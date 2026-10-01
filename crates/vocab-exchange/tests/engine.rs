//! Import and export behavior, one test per rule.

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use vocab_anki::Anki;
use vocab_core::connector::Connector;
use vocab_core::{
    ErrorKind, ItemPatch, ItemSource, LibraryFilter, LibraryView, Verification, VocabItem,
};
use vocab_db::{
    NewItem, add_tag, item_collections, item_tags, list_items, open_in_memory, save_item,
    set_trashed, update_item,
};
use vocab_dictionary::{CedictSource, SqliteDictionary, build_dictionary_db};
use vocab_exchange::{
    ExportRequest, ExportScope, ImportAction, ImportPolicy, SkipReason, apply_export, apply_import,
    content_hash, plan_export, plan_import,
};
use vocab_pleco::Pleco;

fn dictionary() -> SqliteDictionary {
    let artifact = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/dictionary/cedict-sample.u8"),
    )
    .expect("fixture");
    let mut conn = Connection::open_in_memory().expect("memory");
    build_dictionary_db(&mut conn, &CedictSource::default(), &artifact).expect("build");
    SqliteDictionary::from_connection(conn).expect("provider")
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("shouci-exchange-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

fn import(
    conn: &mut Connection,
    connector: &dyn Connector,
    text: &str,
    policy: ImportPolicy,
) -> vocab_exchange::TransferSummary {
    let dict = dictionary();
    let plan = plan_import(
        conn,
        Some(&dict),
        connector,
        text.as_bytes(),
        "/tmp/in.txt",
        policy,
        false,
    )
    .unwrap();
    apply_import(conn, &plan, &content_hash(text.as_bytes())).unwrap()
}

fn all(conn: &Connection) -> Vec<VocabItem> {
    list_items(
        conn,
        &LibraryFilter {
            view: LibraryView::All,
            ..LibraryFilter::default()
        },
    )
    .unwrap()
}

fn find(conn: &Connection, simplified: &str) -> VocabItem {
    vocab_db::list_items(
        conn,
        &LibraryFilter {
            view: LibraryView::All,
            ..LibraryFilter::default()
        },
    )
    .unwrap()
    .into_iter()
    .chain(
        list_items(
            conn,
            &LibraryFilter {
                view: LibraryView::Trash,
                ..LibraryFilter::default()
            },
        )
        .unwrap(),
    )
    .find(|item| item.simplified == simplified)
    .unwrap_or_else(|| panic!("{simplified} not saved"))
}

fn save(conn: &Connection, simplified: &str, pinyin: &str, definition: &str) -> i64 {
    save_traditional(conn, simplified, "", pinyin, definition)
}

fn save_traditional(
    conn: &Connection,
    simplified: &str,
    traditional: &str,
    pinyin: &str,
    definition: &str,
) -> i64 {
    save_item(
        conn,
        &NewItem {
            simplified: simplified.to_owned(),
            traditional: traditional.to_owned(),
            pinyin: pinyin.to_owned(),
            definition: definition.to_owned(),
            notes: String::new(),
            verification: Verification::Confirmed,
            source: ItemSource::manual(),
        },
    )
    .unwrap()
    .item()
    .id
}

const PLECO: &str = "[Greetings]\n你好\tni3 hao3\thello\n[Food]\n米饭\tmi3 fan4\n";

#[test]
fn pleco_import_resolves_words_and_categories_become_collections() {
    let mut conn = open_in_memory().unwrap();
    let summary = import(&mut conn, &Pleco, PLECO, ImportPolicy::Skip);
    assert_eq!(summary.inserted, 2);
    let rice = find(&conn, "米饭");
    assert_eq!(rice.traditional, "米飯", "filled from the dictionary");
    assert_eq!(rice.definition, "cooked rice");
    assert_eq!(rice.verification, Verification::Confirmed);
    assert_eq!(rice.source.id.as_deref(), Some("pleco"));
    assert_eq!(item_collections(&conn, rice.id).unwrap(), vec!["Food"]);
}

#[test]
fn skip_leaves_saved_words_alone_and_reports_differences() {
    let mut conn = open_in_memory().unwrap();
    let id = save(&conn, "你好", "ni3 hao3", "hi there");
    let summary = import(&mut conn, &Pleco, PLECO, ImportPolicy::Skip);
    assert_eq!(summary.inserted, 1);
    assert_eq!(summary.skipped, 1);
    assert_eq!(find(&conn, "你好").definition, "hi there");
    assert!(item_collections(&conn, id).unwrap().is_empty());
    assert!(
        summary.notes.iter().any(|note| note.contains("kept")),
        "{:?}",
        summary.notes
    );
}

#[test]
fn merge_fills_blanks_adds_groups_and_lists_conflicts() {
    let mut conn = open_in_memory().unwrap();
    let id = save(&conn, "你好", "ni3 hao3", "hi there");
    update_item(
        &conn,
        id,
        &ItemPatch {
            notes: Some(String::new()),
            ..ItemPatch::default()
        },
    )
    .unwrap();
    let anki =
        "#separator:tab\n#deck:Lesson 1\n你好\t你好\tni3 hao3\thello\tsay it twice\tgreeting\n";
    let dict = dictionary();
    let plan = plan_import(
        &conn,
        Some(&dict),
        &Anki,
        anki.as_bytes(),
        "/tmp/a.txt",
        ImportPolicy::Merge,
        false,
    )
    .unwrap();
    let ImportAction::Update {
        changes, conflicts, ..
    } = &plan.lines[0].action
    else {
        panic!("expected an update: {:?}", plan.lines[0].action);
    };
    assert_eq!(changes.len(), 1, "notes were blank");
    assert_eq!(changes[0].field, "notes");
    assert_eq!(conflicts.len(), 1, "definitions differ");
    assert_eq!(conflicts[0].kept, "hi there");
    apply_import(&mut conn, &plan, &content_hash(anki.as_bytes())).unwrap();
    let hello = find(&conn, "你好");
    assert_eq!(hello.definition, "hi there");
    assert_eq!(hello.notes, "say it twice");
    assert_eq!(item_tags(&conn, id).unwrap(), vec!["greeting"]);
    assert_eq!(item_collections(&conn, id).unwrap(), vec!["Lesson 1"]);
}

#[test]
fn overwrite_replaces_what_the_file_has_but_never_an_omitted_field() {
    let mut conn = open_in_memory().unwrap();
    let rice = save_traditional(&conn, "米饭", "米飯", "mi3 fan4", "grandma's rice");
    let hello = save(&conn, "你好", "ni3 hao3", "hi there");
    add_tag(&conn, hello, "old").unwrap();
    let summary = import(&mut conn, &Pleco, PLECO, ImportPolicy::Overwrite);
    assert_eq!(summary.updated, 2);
    assert_eq!(find(&conn, "你好").definition, "hello");
    assert_eq!(
        find(&conn, "米饭").definition,
        "grandma's rice",
        "the Pleco line has no definition: nothing to overwrite with"
    );
    assert_eq!(
        item_tags(&conn, hello).unwrap(),
        vec!["old"],
        "Pleco has no tags"
    );
    assert_eq!(item_collections(&conn, rice).unwrap(), vec!["Food"]);
}

#[test]
fn trashed_words_stay_trashed_on_skip_and_come_back_on_merge() {
    let mut conn = open_in_memory().unwrap();
    let id = save(&conn, "你好", "ni3 hao3", "hello");
    set_trashed(&conn, id, true).unwrap();
    let dict = dictionary();
    let plan = plan_import(
        &conn,
        Some(&dict),
        &Pleco,
        PLECO.as_bytes(),
        "/tmp/in.txt",
        ImportPolicy::Skip,
        false,
    )
    .unwrap();
    assert!(matches!(
        plan.lines[0].action,
        ImportAction::Skip {
            reason: SkipReason::InTrash,
            ..
        }
    ));
    import(&mut conn, &Pleco, PLECO, ImportPolicy::Merge);
    assert!(find(&conn, "你好").deleted_at.is_none());
}

#[test]
fn a_word_repeated_in_one_file_is_imported_once() {
    let mut conn = open_in_memory().unwrap();
    let text = "你好\tni3 hao3\thello\n你好\tnǐ hǎo\thello\n";
    let summary = import(&mut conn, &Pleco, text, ImportPolicy::Skip);
    assert_eq!(summary.inserted, 1);
    assert_eq!(summary.skipped, 1);
}

#[test]
fn error_lines_refuse_the_import_unless_forced() {
    let mut conn = open_in_memory().unwrap();
    let text = "你好\tni3 hao3\thello\n\tno characters\n";
    let dict = dictionary();
    let plan = plan_import(
        &conn,
        Some(&dict),
        &Pleco,
        text.as_bytes(),
        "/tmp/bad.txt",
        ImportPolicy::Skip,
        false,
    )
    .unwrap();
    assert!(plan.refused);
    let summary = apply_import(&mut conn, &plan, &content_hash(text.as_bytes())).unwrap();
    assert!(summary.refused);
    assert!(all(&conn).is_empty());

    let forced = plan_import(
        &conn,
        Some(&dict),
        &Pleco,
        text.as_bytes(),
        "/tmp/bad.txt",
        ImportPolicy::Skip,
        true,
    )
    .unwrap();
    let summary = apply_import(&mut conn, &forced, &content_hash(text.as_bytes())).unwrap();
    assert_eq!(summary.inserted, 1);
}

#[test]
fn a_changed_word_or_file_makes_the_plan_stale() {
    let mut conn = open_in_memory().unwrap();
    let id = save(&conn, "你好", "ni3 hao3", "");
    let dict = dictionary();
    let plan = plan_import(
        &conn,
        Some(&dict),
        &Pleco,
        PLECO.as_bytes(),
        "/tmp/in.txt",
        ImportPolicy::Merge,
        false,
    )
    .unwrap();
    let err = apply_import(&mut conn, &plan, "different").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conflict);

    std::thread::sleep(std::time::Duration::from_millis(5));
    update_item(
        &conn,
        id,
        &ItemPatch {
            definition: Some("edited meanwhile".to_owned()),
            ..ItemPatch::default()
        },
    )
    .unwrap();
    let err = apply_import(&mut conn, &plan, &content_hash(PLECO.as_bytes())).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert_eq!(all(&conn).len(), 1, "nothing was written");
}

#[test]
fn without_a_dictionary_incomplete_words_need_review() {
    let mut conn = open_in_memory().unwrap();
    let plan = plan_import(
        &conn,
        None,
        &Pleco,
        PLECO.as_bytes(),
        "/tmp/in.txt",
        ImportPolicy::Skip,
        false,
    )
    .unwrap();
    assert_eq!(plan.counts().unresolved, 1, "米饭 has no definition");
    apply_import(&mut conn, &plan, &content_hash(PLECO.as_bytes())).unwrap();
    assert_eq!(find(&conn, "米饭").verification, Verification::NeedsReview);
    assert_eq!(find(&conn, "你好").verification, Verification::Confirmed);
}

fn export(
    conn: &mut Connection,
    connector: &dyn Connector,
    path: &Path,
    request: &ExportRequest,
) -> vocab_exchange::TransferSummary {
    let dict = dictionary();
    let plan = plan_export(
        conn,
        Some(&dict),
        connector,
        path.to_str().unwrap(),
        request,
    )
    .unwrap();
    apply_export(conn, &plan).unwrap()
}

#[test]
fn export_new_skips_words_the_destination_already_has() {
    let mut conn = open_in_memory().unwrap();
    import(&mut conn, &Pleco, PLECO, ImportPolicy::Skip);
    save(&conn, "学校", "xue2 xiao4", "school");
    let path = scratch("new.txt");
    let first = export(&mut conn, &Pleco, &path, &ExportRequest::default());
    assert_eq!(first.written, 1, "the two Pleco words came from Pleco");
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text, "学校\txue2 xiao4\n", "dictionary definition left out");

    let second = export(&mut conn, &Pleco, &path, &ExportRequest::default());
    assert_eq!(second.written, 0);
    let anki_path = scratch("anki.txt");
    let anki = export(&mut conn, &Anki, &anki_path, &ExportRequest::default());
    assert_eq!(anki.written, 3, "Anki has none of them yet");
}

#[test]
fn export_all_and_selected_and_collection() {
    let mut conn = open_in_memory().unwrap();
    import(&mut conn, &Pleco, PLECO, ImportPolicy::Skip);
    let school = save(&conn, "学校", "xue2 xiao4", "school");
    let path = scratch("all.txt");
    let everything = ExportRequest {
        scope: ExportScope::All,
        ..ExportRequest::default()
    };
    assert_eq!(export(&mut conn, &Pleco, &path, &everything).written, 3);
    let selected = ExportRequest {
        scope: ExportScope::Selected(vec![school]),
        ..ExportRequest::default()
    };
    assert_eq!(export(&mut conn, &Pleco, &path, &selected).written, 1);
    let food = ExportRequest {
        scope: ExportScope::All,
        filter: LibraryFilter {
            collection: Some("food".to_owned()),
            ..LibraryFilter::default()
        },
        ..ExportRequest::default()
    };
    assert_eq!(export(&mut conn, &Pleco, &path, &food).written, 1);
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .starts_with("//food\n")
    );
}

#[test]
fn needs_review_and_trash_stay_home() {
    let mut conn = open_in_memory().unwrap();
    let unsure = save(&conn, "猫", "mao1", "cat");
    update_item(
        &conn,
        unsure,
        &ItemPatch {
            verification: Some(Verification::NeedsReview),
            ..ItemPatch::default()
        },
    )
    .unwrap();
    let gone = save(&conn, "水", "shui3", "water");
    set_trashed(&conn, gone, true).unwrap();
    let path = scratch("review.txt");
    let dict = dictionary();
    let plan = plan_export(
        &conn,
        Some(&dict),
        &Pleco,
        path.to_str().unwrap(),
        &ExportRequest::default(),
    )
    .unwrap();
    assert!(plan.item_ids.is_empty());
    assert_eq!(plan.left_out_needs_review, 1);
    let with_review = ExportRequest {
        include_needs_review: true,
        ..ExportRequest::default()
    };
    assert_eq!(export(&mut conn, &Pleco, &path, &with_review).written, 1);
    let trash = ExportRequest {
        filter: LibraryFilter {
            view: LibraryView::Trash,
            ..LibraryFilter::default()
        },
        ..ExportRequest::default()
    };
    let err = plan_export(&conn, None, &Pleco, path.to_str().unwrap(), &trash).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Invalid);
}

#[test]
fn an_import_source_is_never_overwritten() {
    let mut conn = open_in_memory().unwrap();
    import(&mut conn, &Pleco, PLECO, ImportPolicy::Skip);
    let err = plan_export(
        &conn,
        None,
        &Pleco,
        "/tmp/in.txt",
        &ExportRequest::default(),
    )
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Invalid);
    assert!(err.to_string().contains("imported from"), "{err}");
}

#[test]
fn a_repeated_word_keeps_every_line_s_tags_and_collections() {
    let mut conn = open_in_memory().unwrap();
    let anki =
        "#separator:tab\n你好\t你好\tni3 hao3\thello\t\ttag1\n你好\t你好\tni3 hao3\thi\t\ttag2\n";
    let dict = dictionary();
    let plan = plan_import(
        &conn,
        Some(&dict),
        &Anki,
        anki.as_bytes(),
        "/tmp/r.txt",
        ImportPolicy::Skip,
        false,
    )
    .unwrap();
    assert!(
        plan.issues
            .iter()
            .any(|issue| issue.message.contains("different definition")),
        "{:?}",
        plan.issues
    );
    apply_import(&mut conn, &plan, &content_hash(anki.as_bytes())).unwrap();
    let hello = find(&conn, "你好");
    assert_eq!(hello.definition, "hello", "the first line's definition");
    assert_eq!(item_tags(&conn, hello.id).unwrap(), vec!["tag1", "tag2"]);
}

#[test]
fn blank_cells_never_wipe_saved_text_and_groups_are_only_added() {
    let mut conn = open_in_memory().unwrap();
    let id = save(&conn, "你好", "ni3 hao3", "hello");
    update_item(
        &conn,
        id,
        &ItemPatch {
            notes: Some("my note".to_owned()),
            ..ItemPatch::default()
        },
    )
    .unwrap();
    add_tag(&conn, id, "hsk1").unwrap();
    let anki = "#separator:tab\n你好\t\tni3 hao3\t\t\t\n";
    let summary = import(&mut conn, &Anki, anki, ImportPolicy::Overwrite);
    assert_eq!(summary.updated, 0, "nothing to change");
    let hello = find(&conn, "你好");
    assert_eq!(hello.definition, "hello");
    assert_eq!(hello.notes, "my note");
    assert_eq!(item_tags(&conn, id).unwrap(), vec!["hsk1"]);
}

#[test]
fn our_own_pleco_export_round_trips_as_skips() {
    let mut conn = open_in_memory().unwrap();
    import(
        &mut conn,
        &Pleco,
        "学校[學校]\txue2 xiao4\tschool\n米饭\tmi3 fan4\n",
        ImportPolicy::Skip,
    );
    let path = scratch("own.txt");
    let all = ExportRequest {
        scope: ExportScope::All,
        ..ExportRequest::default()
    };
    export(&mut conn, &Pleco, &path, &all);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("学校[學校]"), "{text}");
    let dict = dictionary();
    let plan = plan_import(
        &conn,
        Some(&dict),
        &Pleco,
        text.as_bytes(),
        "/tmp/own-again.txt",
        ImportPolicy::Skip,
        false,
    )
    .unwrap();
    assert!(
        plan.lines
            .iter()
            .all(|line| matches!(line.action, ImportAction::Skip { .. })),
        "{:?}",
        plan.lines
    );
}

#[test]
fn byte_order_marks_never_reach_saved_words() {
    let mut conn = open_in_memory().unwrap();
    import(
        &mut conn,
        &Pleco,
        "\u{feff}你好\tni3 hao3\thello\n",
        ImportPolicy::Skip,
    );
    assert_eq!(find(&conn, "你好").simplified, "你好");
}

#[test]
fn a_forced_import_records_the_lines_it_skipped() {
    let mut conn = open_in_memory().unwrap();
    let text = "你好\tni3 hao3\thello\n\tno characters\n";
    let plan = plan_import(
        &conn,
        None,
        &Pleco,
        text.as_bytes(),
        "/tmp/forced.txt",
        ImportPolicy::Skip,
        true,
    )
    .unwrap();
    apply_import(&mut conn, &plan, &content_hash(text.as_bytes())).unwrap();
    let rejected: i64 = conn
        .query_row(
            "SELECT count(*) FROM transfer_items WHERE outcome = 'rejected'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rejected, 1);
}

#[test]
fn an_existing_file_is_flagged_and_an_empty_plan_never_truncates() {
    let mut conn = open_in_memory().unwrap();
    save(&conn, "学校", "xue2 xiao4", "school");
    let path = scratch("existing.txt");
    std::fs::write(&path, "keep me").unwrap();
    let all = ExportRequest {
        scope: ExportScope::All,
        ..ExportRequest::default()
    };
    let mut plan = plan_export(&conn, None, &Pleco, path.to_str().unwrap(), &all).unwrap();
    assert!(plan.replaces_existing);
    plan.bytes.clear();
    let err = apply_export(&mut conn, &plan).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Invalid);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep me");
}

#[test]
fn a_tag_edit_after_the_preview_makes_it_stale() {
    let mut conn = open_in_memory().unwrap();
    let id = save(&conn, "你好", "ni3 hao3", "");
    let dict = dictionary();
    let plan = plan_import(
        &conn,
        Some(&dict),
        &Pleco,
        PLECO.as_bytes(),
        "/tmp/in.txt",
        ImportPolicy::Merge,
        false,
    )
    .unwrap();
    add_tag(&conn, id, "edited meanwhile").unwrap();
    let err = apply_import(&mut conn, &plan, &content_hash(PLECO.as_bytes())).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conflict);
}
