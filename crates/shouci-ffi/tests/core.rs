//! The Swift-facing calls, made from Rust: each converts its arguments and
//! results and nothing else. The behavior itself is tested in `shouci-core`.

use std::sync::Arc;

use shouci_core::testing::{SAMPLE_DICTIONARY, install_dictionary, scratch_dir};
use shouci_core::{
    BulkAction, DictionaryStatus, ErrorKind, ExportRequest, ImportAction, ImportPolicy,
    LibraryFilter, LibraryView, ManualWord, SaveOutcome, Verification,
};
use shouci_ffi::{
    Core, CoreConfig, QuickAddResult, ShouciError, default_config, display_definition, tone_marks,
};

struct Library {
    dir: std::path::PathBuf,
    core: Arc<Core>,
}

impl Drop for Library {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn config(dir: &std::path::Path) -> CoreConfig {
    CoreConfig {
        data_dir: dir.to_string_lossy().into_owned(),
        dictionaries_dir: dir.join("dictionaries").to_string_lossy().into_owned(),
        fetch_dictionaries: false,
        legacy_dir: None,
    }
}

/// A library with the sample dictionary loaded.
fn library() -> Library {
    let dir = scratch_dir("ffi");
    let config = config(&dir);
    install_dictionary(
        std::path::Path::new(&config.dictionaries_dir),
        "cc-cedict",
        SAMPLE_DICTIONARY,
    )
    .unwrap();
    let core = Core::open(config).unwrap();
    core.load_dictionaries().unwrap();
    Library { dir, core }
}

fn active() -> LibraryFilter {
    LibraryFilter::default()
}

fn kind(err: &ShouciError) -> ErrorKind {
    let ShouciError::Failed { kind, .. } = err;
    *kind
}

#[test]
fn the_config_comes_back_as_given() {
    let lib = library();
    assert_eq!(lib.core.config(), config(&lib.dir));
    assert_ne!(default_config().data_dir, "");
}

#[test]
fn errors_keep_their_kind_and_message() {
    let dir = scratch_dir("ffi-empty");
    let core = Core::open(config(&dir)).unwrap();
    let err = core
        .search_dictionary("学校".into(), None, None)
        .unwrap_err();
    assert_eq!(err.to_string(), "the dictionary is not loaded");
    assert_eq!(kind(&err), ErrorKind::Unavailable);
    assert_eq!(kind(&core.item(42).unwrap_err()), ErrorKind::NotFound);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn dictionary_status_and_listing() {
    let lib = library();
    assert!(matches!(
        lib.core.dictionary_status(),
        DictionaryStatus::Ready { enabled: 1, .. }
    ));
    let dictionaries = lib.core.dictionaries().unwrap();
    assert_eq!(dictionaries.len(), 1);
    assert_eq!(dictionaries[0].id, "cc-cedict");
    assert!(dictionaries[0].enabled);
}

#[test]
fn search_save_and_find_in_the_library() {
    let lib = library();
    let results = lib
        .core
        .search_dictionary("lvxing".into(), None, Some(5))
        .unwrap();
    let travel = results
        .candidates
        .iter()
        .find(|candidate| candidate.simplified == "旅行")
        .unwrap()
        .clone();
    assert_eq!(travel.pinyin_display, "lǚ xíng");
    assert!(travel.saved.is_none());

    let saved = lib.core.save_candidate(travel).unwrap();
    assert_eq!(saved.outcome, SaveOutcome::Inserted);
    let again = lib
        .core
        .search_dictionary("旅行".into(), None, None)
        .unwrap();
    assert_eq!(
        again.candidates[0].saved.as_ref().unwrap().id,
        saved.item.id
    );

    let found = lib
        .core
        .search_library("travel".into(), active(), None, None)
        .unwrap();
    assert_eq!(found.items[0].id, saved.item.id);
}

#[test]
fn quick_add_saves_or_asks() {
    let lib = library();
    let QuickAddResult::Saved { result } = lib.core.quick_add("学校".into(), None).unwrap()
    else {
        panic!("学校 has one match");
    };
    assert_eq!(result.item.verification, Verification::Confirmed);
    let QuickAddResult::Ambiguous { candidates } =
        lib.core.quick_add("panda".into(), None).unwrap()
    else {
        panic!("two words mean panda");
    };
    assert!(candidates.len() >= 2);
}

#[test]
fn edit_organize_and_trash() {
    let lib = library();
    let word = lib
        .core
        .add_manual(ManualWord {
            simplified: "米饭".into(),
            tags: vec!["food".into()],
            ..ManualWord::default()
        })
        .unwrap()
        .item;
    assert_eq!(word.traditional, "米飯");
    assert_eq!(word.tags, vec!["food"]);

    let edited = lib
        .core
        .update_item(
            word.id,
            shouci_core::ItemPatch {
                notes: Some("lunch".into()),
                ..shouci_core::ItemPatch::default()
            },
        )
        .unwrap();
    assert_eq!(edited.notes, "lunch");

    lib.core.create_collection("Week 1".into()).unwrap();
    lib.core
        .bulk(vec![word.id], BulkAction::AddToCollection("Week 1".into()))
        .unwrap();
    assert_eq!(lib.core.collections().unwrap()[0].count, 1);
    assert_eq!(lib.core.tags().unwrap()[0].name, "food");

    lib.core.bulk(vec![word.id], BulkAction::Trash).unwrap();
    let trash = LibraryFilter {
        view: LibraryView::Trash,
        ..LibraryFilter::default()
    };
    assert_eq!(lib.core.list_items(trash).unwrap().len(), 1);
    assert_eq!(lib.core.empty_trash().unwrap().changed, 1);
}

#[test]
fn a_saved_word_in_another_dictionary() {
    let lib = library();
    let QuickAddResult::Saved { result } = lib.core.quick_add("学校".into(), None).unwrap()
    else {
        panic!("学校 has one match");
    };
    let entries = lib
        .core
        .lookup_in(result.item.id, "cc-cedict".into())
        .unwrap();
    assert!(entries[0].same_word);
    let item = lib
        .core
        .use_definition(result.item.id, "cc-cedict".into())
        .unwrap();
    assert_eq!(item.definition, "school");
}

#[test]
fn import_and_export_through_previews() {
    let lib = library();
    let source = lib.dir.join("from-pleco.txt");
    std::fs::write(&source, "[Food]\n米饭\tmi3 fan4\n你好\tni3 hao3\thello\n").unwrap();

    let preview = lib
        .core
        .preview_import(
            source.to_string_lossy().into_owned(),
            "pleco".into(),
            ImportPolicy::Skip,
            false,
        )
        .unwrap();
    let view = preview.view();
    assert_eq!(view.counts.inserts, 2);
    assert!(!view.refused);
    assert!(matches!(view.lines[0].action, ImportAction::Insert { .. }));
    assert_eq!(lib.core.apply_import(preview).unwrap().inserted, 2);

    let out = lib.dir.join("to-anki.txt");
    let preview = lib
        .core
        .preview_export(
            out.to_string_lossy().into_owned(),
            "anki".into(),
            ExportRequest::default(),
        )
        .unwrap();
    assert_eq!(preview.view().words, vec!["米饭", "你好"]);
    assert_eq!(lib.core.apply_export(preview).unwrap().written, 2);
    assert!(std::fs::read_to_string(out).unwrap().contains("米饭"));
}

#[test]
fn the_data_version_moves_with_writes() {
    let lib = library();
    let before = lib.core.data_version().unwrap();
    lib.core.quick_add("学校".into(), None).unwrap();
    assert_ne!(lib.core.data_version().unwrap(), before);
}

#[test]
fn pinyin_is_shown_with_tone_marks() {
    assert_eq!(tone_marks("xue2 xiao4"), "xué xiào");
}

#[test]
fn definitions_are_cleaned_up_for_reading() {
    let shown = display_definition("CL:個|个[ge4]");
    assert!(!shown.contains("[ge4]"), "{shown}");
    assert!(shown.contains("gè"), "{shown}");
}
