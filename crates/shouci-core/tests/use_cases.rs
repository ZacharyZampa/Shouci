//! The product's use cases, end to end through `Shouci`, one test each.
//! Each test reads as Given / When / Then.

use std::path::Path;

use shouci_core::testing::{empty_sandbox, install_dictionary, sandbox, scratch_dir};
use shouci_core::{
    BulkAction, Config, DictionaryStatus, ErrorKind, ExportRequest, ExportScope, FrequencyBand,
    ImportAction, ImportPolicy, ItemPatch, LibraryFilter, LibraryView, Lifecycle, ManualWord,
    MatchBasis, QueryKind, QuickAdd, SaveOutcome, Shouci, SmartCollectionView, SourceKind,
    Verification,
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
fn quick_add_keeps_unknown_chinese_for_review() {
    let shouci = sandbox();
    let result = saved(shouci.quick_add("蚌埠住了", None).unwrap());
    assert_eq!(result.item.verification, Verification::NeedsReview);
    assert_eq!(result.item.simplified, "蚌埠住了");
    assert_eq!(result.item.definition, "", "no invented definition");
}

#[test]
fn quick_add_refuses_unknown_text_that_is_not_chinese() {
    // A typo is not a word to fill in later.
    let shouci = sandbox();
    let err = shouci.quick_add("zzzqqq", None).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert_eq!(heads(&shouci, &active()), [] as [String; 0]);
}

#[test]
fn an_edit_with_a_refused_tag_changes_nothing() {
    let shouci = sandbox();
    let id = saved(shouci.quick_add("学校", None).unwrap()).item.id;
    let before = shouci.item(id).unwrap();
    let err = shouci
        .edit_item(
            id,
            &ItemPatch {
                definition: Some("place of learning".to_owned()),
                ..ItemPatch::default()
            },
            &[
                BulkAction::AddTags(vec!["fine".to_owned()]),
                BulkAction::AddTags(vec!["bad\tname".to_owned()]),
            ],
        )
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Invalid);
    assert_eq!(shouci.item(id).unwrap(), before);
    let edited = shouci
        .edit_item(
            id,
            &ItemPatch {
                definition: Some("place of learning".to_owned()),
                ..ItemPatch::default()
            },
            &[BulkAction::AddTags(vec!["fine".to_owned()])],
        )
        .unwrap();
    assert_eq!(edited.definition, "place of learning");
    assert_eq!(edited.tags, ["fine"]);
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
fn words_not_yet_in_a_collection_have_a_view_of_their_own() {
    let shouci = sandbox();
    let school = saved(shouci.quick_add("学校", None).unwrap()).item.id;
    saved(shouci.quick_add("旅行", None).unwrap());
    shouci
        .bulk(
            &[school],
            &BulkAction::AddToCollection("Lesson 1".to_owned()),
        )
        .unwrap();
    let unsorted = LibraryFilter {
        no_collection: true,
        ..LibraryFilter::default()
    };
    assert_eq!(heads(&shouci, &unsorted), ["旅行"]);
}

#[test]
fn a_tag_or_collection_merges_into_another() {
    let shouci = sandbox();
    let school = saved(shouci.quick_add("学校", None).unwrap()).item.id;
    let travel = saved(shouci.quick_add("旅行", None).unwrap()).item.id;
    shouci
        .bulk(&[school], &BulkAction::AddToCollection("Week_1".to_owned()))
        .unwrap();
    shouci
        .bulk(&[travel], &BulkAction::AddToCollection("Week 1".to_owned()))
        .unwrap();
    shouci
        .bulk(
            &[school, travel],
            &BulkAction::AddTags(vec!["HSK1".to_owned()]),
        )
        .unwrap();
    shouci
        .bulk(&[school], &BulkAction::AddTags(vec!["HSK 1".to_owned()]))
        .unwrap();
    assert_eq!(
        shouci
            .rename_collection("Week_1", "week 1")
            .unwrap_err()
            .kind(),
        ErrorKind::Conflict,
        "renaming onto a name in use is a merge, asked for as one"
    );

    shouci.merge_collections("Week_1", "Week 1").unwrap();
    shouci.merge_tags("hsk1", "HSK 1").unwrap();

    let collections = shouci.collections().unwrap();
    assert_eq!(collections.len(), 1);
    assert_eq!(
        (collections[0].name.as_str(), collections[0].count),
        ("Week 1", 2)
    );
    assert_eq!(shouci.item(travel).unwrap().tags, ["HSK 1"]);
    assert_eq!(shouci.tags().unwrap().len(), 1);
    assert_eq!(
        shouci.merge_tags("missing", "HSK 1").unwrap_err().kind(),
        ErrorKind::NotFound
    );
}

/// Installs the fixture dictionary with frequency and HSK layers: 水 and
/// 学校 are HSK 1, 猫 and 旅行 HSK 2, and all four are in the top 1,000.
fn install_ranked_dictionary(shouci: &Shouci) {
    let fixture = |name: &str| {
        std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/dictionary")
                .join(name),
        )
        .unwrap()
    };
    let (frequency, hsk) = (fixture("frequency-sample.txt"), fixture("hsk-sample.txt"));
    let path =
        vocab_dictionary::catalog::dictionary_path(&shouci.config().dictionaries_dir, "cc-cedict");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    vocab_dictionary::build_dictionary_db_with_layers(
        &mut rusqlite::Connection::open(&path).unwrap(),
        &vocab_dictionary::CedictSource::new("cc-cedict", "test"),
        &fixture("cedict-sample.u8"),
        &[
            (
                &vocab_dictionary::FrequencySource::default(),
                frequency.as_slice(),
            ),
            (&vocab_dictionary::HskSource::default(), hsk.as_slice()),
        ],
    )
    .unwrap();
}

#[test]
fn saved_words_show_how_common_they_are_once_the_dictionary_loads() {
    let shouci = empty_sandbox();
    install_ranked_dictionary(&shouci);
    let cat = shouci
        .add_manual(&ManualWord {
            simplified: "猫".to_owned(),
            pinyin: Some("mao1".to_owned()),
            definition: Some("cat".to_owned()),
            ..ManualWord::default()
        })
        .unwrap()
        .item;
    assert_eq!(
        (cat.frequency_rank, cat.hsk_rank),
        (None, None),
        "no dictionary yet"
    );

    shouci.load_dictionaries().unwrap();

    let cat = shouci.item(cat.id).unwrap();
    assert_eq!((cat.frequency_rank, cat.hsk_rank), (Some(3), Some(2)));
    assert_eq!(cat.frequency_band, FrequencyBand::Top1000);
    let listed = shouci.list_items(&active()).unwrap();
    assert_eq!(listed[0].hsk_rank, Some(2));
    let world = saved(shouci.quick_add("世界", None).unwrap()).item;
    assert_eq!(
        (world.frequency_rank, world.hsk_rank),
        (None, None),
        "in neither list"
    );
    assert_eq!(world.frequency_band, FrequencyBand::Unlisted);
}

#[test]
fn words_are_filtered_by_hsk_level_and_frequency() {
    let shouci = empty_sandbox();
    install_ranked_dictionary(&shouci);
    shouci.load_dictionaries().unwrap();
    for word in ["水", "猫", "旅行", "世界"] {
        saved(shouci.quick_add(word, None).unwrap());
    }
    let by = |hsk_levels: Vec<u64>, frequency_bands: Vec<FrequencyBand>| LibraryFilter {
        hsk_levels,
        frequency_bands,
        ..LibraryFilter::default()
    };
    assert_eq!(heads(&shouci, &by(vec![2], vec![])), ["旅行", "猫"]);
    assert_eq!(
        heads(&shouci, &by(vec![1, 2], vec![])),
        ["旅行", "猫", "水"]
    );
    assert_eq!(heads(&shouci, &by(vec![0], vec![])), ["世界"], "no level");
    assert_eq!(
        heads(&shouci, &by(vec![], vec![FrequencyBand::Unlisted])),
        ["世界"]
    );
    assert_eq!(
        heads(&shouci, &by(vec![2], vec![FrequencyBand::Top1000])),
        ["旅行", "猫"]
    );
    let found = shouci
        .search_library("lv3", &by(vec![2], vec![]), None, None)
        .unwrap();
    assert_eq!(found.items.len(), 1, "search narrows by level too");

    let ids = shouci.matching_ids(&by(vec![2], vec![])).unwrap();
    let listed: Vec<i64> = shouci
        .list_items(&by(vec![2], vec![]))
        .unwrap()
        .iter()
        .map(|item| item.id)
        .collect();
    assert_eq!(ids, listed, "the same words, newest first");

    let dir = scratch_dir("ranked-export");
    let request = ExportRequest {
        scope: ExportScope::New,
        filter: by(vec![1], vec![]),
        ..ExportRequest::default()
    };
    let plan = shouci
        .preview_export(&dir.join("hsk1.txt"), "pleco", &request)
        .unwrap();
    assert_eq!(plan.words, ["水"], "exports keep to the level");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn smart_collections_keep_a_filter_and_find_its_words_each_time() {
    let shouci = empty_sandbox();
    install_ranked_dictionary(&shouci);
    shouci.load_dictionaries().unwrap();
    let cat = saved(shouci.quick_add("猫", None).unwrap()).item.id;
    let water = saved(shouci.quick_add("水", None).unwrap()).item.id;
    let hsk2 = LibraryFilter {
        hsk_levels: vec![2],
        without_tags: vec!["drilled".to_owned()],
        ..LibraryFilter::default()
    };

    let smart = shouci.create_smart_collection("To Drill", &hsk2).unwrap();
    assert_eq!(smart.item_ids, vec![cat]);
    let travel = saved(shouci.quick_add("旅行", None).unwrap()).item.id;
    shouci
        .bulk(&[cat], &BulkAction::AddTags(vec!["Drilled".to_owned()]))
        .unwrap();
    let smart = shouci.smart_collection("to drill").unwrap();
    assert_eq!(smart.item_ids, vec![travel], "worked out when looked at");

    shouci.rename_tag("Drilled", "done").unwrap();
    let followed = shouci.smart_collection("To Drill").unwrap().filter;
    assert_eq!(
        followed.without_tags,
        ["done"],
        "the filter follows the tag"
    );
    let smart = shouci
        .update_smart_collection(
            "To Drill",
            &LibraryFilter {
                hsk_levels: vec![1, 2],
                ..followed
            },
        )
        .unwrap();
    assert_eq!(smart.item_ids, vec![travel, water]);
    shouci
        .rename_smart_collection("To Drill", "Review")
        .unwrap();
    assert_eq!(
        shouci
            .smart_collections()
            .unwrap()
            .iter()
            .map(|smart| smart.name.as_str())
            .collect::<Vec<_>>(),
        ["Review"]
    );

    let taken = shouci.create_smart_collection("review", &hsk2).unwrap_err();
    assert_eq!(taken.kind(), ErrorKind::Conflict);
    let trash = LibraryFilter {
        view: LibraryView::Trash,
        ..LibraryFilter::default()
    };
    let refused = shouci.create_smart_collection("Bin", &trash).unwrap_err();
    assert_eq!(refused.kind(), ErrorKind::Invalid);
    let level = LibraryFilter {
        hsk_levels: vec![8],
        ..LibraryFilter::default()
    };
    let refused = shouci.create_smart_collection("Eight", &level).unwrap_err();
    assert_eq!(refused.kind(), ErrorKind::Invalid);

    shouci.delete_smart_collection("Review").unwrap();
    assert_eq!(
        shouci.smart_collections().unwrap(),
        Vec::<SmartCollectionView>::new()
    );
    assert_eq!(heads(&shouci, &active()).len(), 3, "its words stay");
    assert_eq!(
        shouci.smart_collection("Review").unwrap_err().kind(),
        ErrorKind::NotFound
    );
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

// --- Undo -----------------------------------------------------------------

/// Runs `change` on `ids` as a frontend does for ⌘Z: snapshots before and
/// after, returned in that order.
fn recorded(
    shouci: &Shouci,
    ids: &[i64],
    change: impl FnOnce(&Shouci),
) -> (shouci_core::LibrarySnapshot, shouci_core::LibrarySnapshot) {
    let before = shouci.snapshot(ids).unwrap();
    change(shouci);
    (before, shouci.snapshot(ids).unwrap())
}

#[test]
fn a_bulk_change_is_undone_and_redone() {
    let shouci = sandbox();
    let school = saved(shouci.quick_add("学校", None).unwrap()).item.id;
    let travel = saved(shouci.quick_add("旅行", None).unwrap()).item.id;
    shouci
        .bulk(&[school], &BulkAction::AddTags(vec!["places".to_owned()]))
        .unwrap();
    let (before, after) = recorded(&shouci, &[school, travel], |shouci| {
        shouci
            .bulk(&[school, travel], &BulkAction::Archive)
            .unwrap();
        shouci
            .bulk(
                &[school, travel],
                &BulkAction::AddTags(vec!["week 3".to_owned()]),
            )
            .unwrap();
    });

    shouci.restore(&before, &after).unwrap();
    let school_now = shouci.item(school).unwrap();
    assert_eq!(school_now.lifecycle, Lifecycle::Active);
    assert_eq!(school_now.tags, ["places"]);
    assert!(
        shouci
            .tags()
            .unwrap()
            .iter()
            .all(|tag| tag.name != "week 3"),
        "the tag the change created goes with it"
    );

    shouci.restore(&after, &before).unwrap();
    let travel_now = shouci.item(travel).unwrap();
    assert_eq!(travel_now.lifecycle, Lifecycle::Archived);
    assert_eq!(travel_now.tags, ["week 3"]);
    assert_eq!(
        travel_now.archived_at,
        after
            .words
            .iter()
            .find(|w| w.id == travel)
            .unwrap()
            .archived_at,
        "the archive date comes back too"
    );
}

#[test]
fn edits_merges_renames_and_deletes_are_undone() {
    let shouci = sandbox();
    let school = saved(shouci.quick_add("学校", None).unwrap()).item.id;
    let travel = saved(shouci.quick_add("旅行", None).unwrap()).item.id;
    shouci
        .bulk(&[school], &BulkAction::AddTags(vec!["HSK1".to_owned()]))
        .unwrap();
    shouci
        .bulk(&[travel], &BulkAction::AddTags(vec!["HSK 1".to_owned()]))
        .unwrap();
    shouci
        .bulk(
            &[school, travel],
            &BulkAction::AddToCollection("Week 1".to_owned()),
        )
        .unwrap();
    shouci.create_collection("Empty").unwrap();
    let ids = [school, travel];

    let (before, after) = recorded(&shouci, &[school], |shouci| {
        shouci
            .update_item(
                school,
                &ItemPatch {
                    definition: Some("my school".to_owned()),
                    notes: Some("near home".to_owned()),
                    verification: Some(Verification::NeedsReview),
                    ..ItemPatch::default()
                },
            )
            .unwrap();
    });
    shouci.restore(&before, &after).unwrap();
    let school_now = shouci.item(school).unwrap();
    assert_eq!(school_now.definition, "school");
    assert_eq!(school_now.notes, "");
    assert_eq!(school_now.verification, Verification::Confirmed);

    let (before, after) = recorded(&shouci, &[travel], |shouci| {
        shouci.merge_tags("HSK 1", "HSK1").unwrap();
    });
    shouci.restore(&before, &after).unwrap();
    assert_eq!(shouci.item(travel).unwrap().tags, ["HSK 1"]);
    assert_eq!(shouci.item(school).unwrap().tags, ["HSK1"]);
    shouci.restore(&after, &before).unwrap();
    assert_eq!(shouci.tags().unwrap().len(), 1, "merged again");
    shouci.restore(&before, &after).unwrap();

    let (before, after) = recorded(&shouci, &ids, |shouci| {
        shouci.rename_collection("Week 1", "week 1").unwrap();
    });
    shouci.restore(&before, &after).unwrap();
    assert_eq!(
        shouci.item(school).unwrap().collections,
        ["Week 1"],
        "case and all"
    );

    let (before, after) = recorded(&shouci, &ids, |shouci| {
        shouci.delete_collection("Week 1").unwrap();
        shouci.delete_collection("Empty").unwrap();
    });
    shouci.restore(&before, &after).unwrap();
    let names: Vec<String> = shouci
        .collections()
        .unwrap()
        .into_iter()
        .map(|c| c.name)
        .collect();
    assert_eq!(
        names,
        ["Empty", "Week 1"],
        "an empty collection comes back too"
    );
    assert_eq!(shouci.item(travel).unwrap().collections, ["Week 1"]);

    let (before, after) = recorded(&shouci, &ids, |shouci| {
        shouci.create_collection("Week 2").unwrap();
        shouci
            .bulk(&ids, &BulkAction::AddToCollection("Week 2".to_owned()))
            .unwrap();
    });
    shouci.restore(&before, &after).unwrap();
    assert!(
        shouci
            .collections()
            .unwrap()
            .iter()
            .all(|c| c.name != "Week 2"),
        "a collection the change made goes"
    );
}

#[test]
fn undoing_several_changes_in_turn_works() {
    let shouci = sandbox();
    let school = saved(shouci.quick_add("学校", None).unwrap()).item.id;
    let first = recorded(&shouci, &[school], |shouci| {
        shouci.bulk(&[school], &BulkAction::Archive).unwrap();
    });
    let second = recorded(&shouci, &[school], |shouci| {
        shouci
            .bulk(&[school], &BulkAction::AddTags(vec!["later".to_owned()]))
            .unwrap();
    });
    shouci.restore(&second.0, &second.1).unwrap();
    shouci.restore(&first.0, &first.1).unwrap();
    assert_eq!(shouci.item(school).unwrap().lifecycle, Lifecycle::Active);
    shouci.restore(&first.1, &first.0).unwrap();
    shouci.restore(&second.1, &second.0).unwrap();
    let redone = shouci.item(school).unwrap();
    assert_eq!(
        (redone.lifecycle, redone.tags.as_slice()),
        (Lifecycle::Archived, &["later".to_owned()][..])
    );
}

#[test]
fn a_word_changed_or_deleted_since_is_never_overwritten() {
    let shouci = sandbox();
    let school = saved(shouci.quick_add("学校", None).unwrap()).item.id;
    let (before, after) = recorded(&shouci, &[school], |shouci| {
        shouci.bulk(&[school], &BulkAction::Archive).unwrap();
    });
    shouci
        .update_item(
            school,
            &ItemPatch {
                notes: Some("written by shouci meanwhile".to_owned()),
                ..ItemPatch::default()
            },
        )
        .unwrap();
    let err = shouci.restore(&before, &after).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert!(err.to_string().contains("changed in the meantime"), "{err}");
    let kept = shouci.item(school).unwrap();
    assert_eq!(kept.notes, "written by shouci meanwhile");
    assert_eq!(kept.lifecycle, Lifecycle::Archived, "nothing changed");
    let unknown = shouci.restore(&before, &shouci_core::LibrarySnapshot::default());
    assert_eq!(
        unknown.unwrap_err().kind(),
        ErrorKind::Conflict,
        "nor one the change's snapshot doesn't have"
    );

    let travel = saved(shouci.quick_add("旅行", None).unwrap()).item.id;
    let (before, after) = recorded(&shouci, &[travel], |shouci| {
        shouci.bulk(&[travel], &BulkAction::Trash).unwrap();
    });
    shouci.empty_trash().unwrap();
    let err = shouci.restore(&before, &after).unwrap_err();
    assert!(err.to_string().contains("deleted for good"), "{err}");
}

#[test]
fn undoing_a_rename_or_merge_puts_smart_filters_back_too() {
    let shouci = sandbox();
    let school = saved(shouci.quick_add("学校", None).unwrap()).item.id;
    let travel = saved(shouci.quick_add("旅行", None).unwrap()).item.id;
    shouci
        .bulk(&[school], &BulkAction::AddTags(vec!["drill".to_owned()]))
        .unwrap();
    shouci
        .bulk(&[travel], &BulkAction::AddTags(vec!["Drill 2".to_owned()]))
        .unwrap();
    let fresh = LibraryFilter {
        without_tags: vec!["drill".to_owned(), "Drill 2".to_owned()],
        ..LibraryFilter::default()
    };
    shouci.create_smart_collection("Fresh", &fresh).unwrap();
    let saved_filter = |shouci: &Shouci| shouci.smart_collection("Fresh").unwrap().filter;

    let (before, after) = recorded(&shouci, &[school], |shouci| {
        shouci.rename_tag("drill", "drilled").unwrap();
    });
    assert_eq!(saved_filter(&shouci).without_tags, ["drilled", "Drill 2"]);
    shouci.restore(&before, &after).unwrap();
    assert_eq!(saved_filter(&shouci), fresh, "renamed back");
    assert_eq!(shouci.item(school).unwrap().tags, ["drill"]);
    shouci.restore(&after, &before).unwrap();
    assert_eq!(saved_filter(&shouci).without_tags, ["drilled", "Drill 2"]);
    shouci.restore(&before, &after).unwrap();

    let (before, after) = recorded(&shouci, &[travel], |shouci| {
        shouci.merge_tags("Drill 2", "drill").unwrap();
    });
    assert_eq!(saved_filter(&shouci).without_tags, ["drill"]);
    shouci.restore(&before, &after).unwrap();
    assert_eq!(saved_filter(&shouci), fresh, "the merge is taken back");

    let (before, after) = recorded(&shouci, &[school], |shouci| {
        shouci.collection_from_tag("drill").unwrap();
    });
    assert_eq!(saved_filter(&shouci).without_collections, ["drill"]);
    shouci.restore(&before, &after).unwrap();
    assert_eq!(saved_filter(&shouci), fresh, "a tag again");

    let (before, after) = recorded(&shouci, &[school], |shouci| {
        shouci.rename_tag("drill", "drilled").unwrap();
    });
    let edited = LibraryFilter {
        hsk_levels: vec![1],
        ..saved_filter(&shouci)
    };
    shouci.update_smart_collection("Fresh", &edited).unwrap();
    let err = shouci.restore(&before, &after).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conflict, "a filter edited since");
    assert_eq!(saved_filter(&shouci), edited, "nothing changed");
    assert_eq!(shouci.item(school).unwrap().tags, ["drilled"]);
}

#[test]
fn smart_collection_changes_are_undone_unless_changed_since() {
    let shouci = sandbox();
    let hsk = |level| LibraryFilter {
        hsk_levels: vec![level],
        ..LibraryFilter::default()
    };
    let saved_filters = |shouci: &Shouci| shouci.snapshot(&[]).unwrap().smart_filters;

    // Created, changed, renamed, deleted: each undone and redone
    let changes: [&dyn Fn(&Shouci); 4] = [
        &|shouci| drop(shouci.create_smart_collection("Drill", &hsk(1)).unwrap()),
        &|shouci| drop(shouci.update_smart_collection("Drill", &hsk(2)).unwrap()),
        &|shouci| shouci.rename_smart_collection("Drill", "drill").unwrap(),
        &|shouci| shouci.delete_smart_collection("drill").unwrap(),
    ];
    for change in changes {
        let (before, after) = recorded(&shouci, &[], change);
        shouci.restore(&before, &after).unwrap();
        assert_eq!(saved_filters(&shouci), before.smart_filters, "undone");
        shouci.restore(&after, &before).unwrap();
        assert_eq!(saved_filters(&shouci), after.smart_filters, "redone");
    }

    // Changed by `shouci` since: the undo changes nothing
    let (before, after) = recorded(&shouci, &[], |shouci| {
        shouci.create_smart_collection("Drill", &hsk(1)).unwrap();
    });
    shouci.update_smart_collection("Drill", &hsk(3)).unwrap();
    let err = shouci.restore(&before, &after).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert!(err.to_string().contains("Drill changed"), "{err}");
    assert_eq!(shouci.smart_collection("Drill").unwrap().filter, hsk(3));
}

#[test]
fn an_add_is_taken_back_and_redone_as_the_same_word() {
    let shouci = sandbox();
    let (before, after) = {
        let before = shouci.snapshot(&[]).unwrap();
        let added = shouci
            .add_manual(&ManualWord {
                simplified: "米饭".to_owned(),
                tags: vec!["new".to_owned()],
                ..ManualWord::default()
            })
            .unwrap();
        (before, shouci.snapshot(&[added.item.id]).unwrap())
    };
    let word = after.words[0].clone();

    // Undone: the word goes for good, and so does the tag it brought
    shouci.restore(&before, &after).unwrap();
    assert_eq!(
        shouci.item(word.id).unwrap_err().kind(),
        ErrorKind::NotFound
    );
    assert_eq!(shouci.tags().unwrap(), Vec::new());

    // Redone: the same word, with its id and when it was added
    shouci.restore(&after, &before).unwrap();
    let back = shouci.item(word.id).unwrap();
    assert_eq!(
        (
            back.simplified.as_str(),
            back.created_at.as_str(),
            back.tags.as_slice()
        ),
        ("米饭", word.created_at.as_str(), &["new".to_owned()][..])
    );
    assert!(back.rev > word.rev, "a redo is a change too");

    // Edited since: taking the add back would lose the edit
    shouci
        .update_item(
            word.id,
            &ItemPatch {
                notes: Some("from shouci".to_owned()),
                ..ItemPatch::default()
            },
        )
        .unwrap();
    let err = shouci.restore(&before, &after).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert_eq!(shouci.item(word.id).unwrap().notes, "from shouci");
}

#[test]
fn a_word_typed_in_again_is_found_first_so_its_save_can_be_undone() {
    let shouci = sandbox();
    let word = ManualWord {
        simplified: "生词".to_owned(),
        ..ManualWord::default()
    };
    assert_eq!(shouci.manual_match(&word).unwrap(), None);
    let id = shouci.add_manual(&word).unwrap().item.id;
    shouci.bulk(&[id], &BulkAction::Trash).unwrap();
    let trashed = shouci.item(id).unwrap();

    // Typed in again: found in the trash before saving brings it back
    assert_eq!(shouci.manual_match(&word).unwrap(), Some(id));
    let (before, after) = recorded(&shouci, &[id], |shouci| {
        let back = shouci.add_manual(&word).unwrap();
        assert_eq!(back.outcome, SaveOutcome::Restored);
    });

    // so undoing puts it back in the trash as it was
    shouci.restore(&before, &after).unwrap();
    let undone = shouci.item(id).unwrap();
    assert_eq!(
        (undone.lifecycle, undone.deleted_at),
        (Lifecycle::Trashed, trashed.deleted_at)
    );
}

#[test]
fn a_word_deleted_for_good_while_a_change_ran_is_not_brought_back() {
    let shouci = sandbox();
    let school = saved(shouci.quick_add("学校", None).unwrap()).item.id;
    let (before, after) = recorded(&shouci, &[school], |shouci| {
        // Emptied from the Trash by `shouci` while the change runs
        shouci.bulk(&[school], &BulkAction::Trash).unwrap();
        shouci.empty_trash().unwrap();
    });
    let err = shouci.restore(&before, &after).unwrap_err();
    assert!(err.to_string().contains("deleted for good"), "{err}");
    assert_eq!(shouci.item(school).unwrap_err().kind(), ErrorKind::NotFound);
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
fn a_word_changed_after_the_preview_is_not_imported() {
    let shouci = sandbox();
    let id = saved(shouci.quick_add("你好", None).unwrap()).item.id;
    let dir = scratch_dir("word-changed");
    let source = dir.join("in.txt");
    std::fs::write(&source, "// Lesson 1\n你好\tni3 hao3\thello\n").unwrap();
    let plan = shouci
        .preview_import(&source, "pleco", ImportPolicy::Merge, false)
        .unwrap();
    shouci
        .update_item(
            id,
            &ItemPatch {
                notes: Some("edited meanwhile".to_owned()),
                ..ItemPatch::default()
            },
        )
        .unwrap();
    let err = shouci.apply_import(&plan).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conflict);
    let word = shouci.item(id).unwrap();
    assert_eq!(word.notes, "edited meanwhile");
    assert!(word.collections.is_empty(), "nothing was imported");
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
        let found = shouci
            .detect_import(path, ImportPolicy::Skip, false)
            .unwrap();
        assert_eq!(found.plan.connector_id, expected, "{}", path.display());
        assert!(found.unambiguous, "{}", path.display());
    }
    // An empty file reads as either, so neither is sure
    let empty = dir.join("empty.txt");
    std::fs::write(&empty, "").unwrap();
    let found = shouci
        .detect_import(&empty, ImportPolicy::Skip, false)
        .unwrap();
    assert!(!found.unambiguous);
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

/// The app and the CLI write at the same moment: each waits its turn, and
/// nothing is lost. (A transaction that reads before taking the write lock
/// fails here at once instead of waiting.)
#[test]
fn writers_in_other_processes_wait_their_turn() {
    let shouci = empty_sandbox();
    let mut config = shouci.config().clone();
    config.legacy_dir = None;
    let writers: Vec<_> = (0..6u32)
        .map(|writer| {
            let config = config.clone();
            std::thread::spawn(move || {
                let other = Shouci::open(config).unwrap();
                for n in 0..25u32 {
                    let word = char::from_u32(0x4E00 + writer * 100 + n).unwrap();
                    let id = other
                        .add_manual(&ManualWord {
                            simplified: word.to_string(),
                            ..ManualWord::default()
                        })
                        .unwrap()
                        .item
                        .id;
                    other
                        .bulk(&[id], &BulkAction::AddTags(vec!["busy".to_owned()]))
                        .unwrap();
                }
            })
        })
        .collect();
    for writer in writers {
        writer.join().unwrap();
    }
    assert_eq!(shouci.list_items(&active()).unwrap().len(), 150);
    assert_eq!(shouci.tags().unwrap()[0].count, 150);
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
    // Smart collections keep filters in this shape in user.db.
    let filter: LibraryFilter = serde_json::from_str(
        r#"{"hsk_levels": [4, 0], "frequency_bands": ["top1000", "to5000", "to10000",
            "beyond10000", "unlisted"], "without_tags": ["drilled"], "added_within_days": 30}"#,
    )
    .unwrap();
    assert_eq!(filter.frequency_bands, FrequencyBand::ALL);
    assert_eq!(filter.view, LibraryView::Active, "left out, as before");
    assert_eq!(
        serde_json::to_value(&filter).unwrap()["frequency_bands"][0],
        "top1000"
    );
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
    let typed = ManualWord {
        simplified: "学校".to_owned(),
        ..ManualWord::default()
    };
    assert_eq!(
        shouci.manual_match(&typed).unwrap(),
        Some(placeholder.item.id),
        "the word typed in now completes it too"
    );
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
