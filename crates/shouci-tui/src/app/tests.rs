use std::path::Path;

use crossterm::event::KeyEventKind;
use shouci_core::testing::{Sandbox, sandbox, scratch_dir};
use shouci_core::{LibraryFilter, Verification, expand_tilde};

use super::*;

fn press(app: &mut App<'_>, code: KeyCode) {
    app.on_key(&KeyEvent::new(code, KeyModifiers::NONE))
        .expect("key handled");
}

fn ctrl(app: &mut App<'_>, code: KeyCode) {
    app.on_key(&KeyEvent::new(code, KeyModifiers::CONTROL))
        .expect("key handled");
}

fn type_text(app: &mut App<'_>, text: &str) {
    for ch in text.chars() {
        press(app, KeyCode::Char(ch));
    }
}

/// Saved words outside the trash, newest first.
fn saved_heads(library: &Sandbox) -> Vec<String> {
    library
        .list_items(&LibraryFilter::default())
        .unwrap()
        .into_iter()
        .map(|item| item.simplified)
        .collect()
}

fn search_and_save(app: &mut App<'_>, word: &str) {
    if !app.query.is_empty() {
        // Esc on a non-empty query clears it (never quits here).
        press(app, KeyCode::Esc);
    }
    type_text(app, word);
    // Live search already populated results; Enter saves the selection.
    press(app, KeyCode::Enter);
    assert!(
        app.status.starts_with("saved ") || app.status.starts_with("already saved "),
        "unexpected status after Enter: {:?}",
        app.status
    );
}

fn headword(row: &Row) -> &str {
    match row {
        Row::Candidate(candidate) => &candidate.simplified,
        Row::AsTyped(text) => text,
    }
}

// --- Search ----------------------------------------------------------

#[test]
fn typing_populates_results_live() {
    let library = sandbox();
    let mut app = App::new(&library);
    type_text(&mut app, "school");
    // No Enter needed: results arrive as you type.
    assert_eq!(app.rows.len(), 1);
    assert_eq!(headword(&app.rows[0]), "学校");
    assert_eq!(app.list_state.selected(), Some(0));
    assert_eq!(app.mode_label(), "English · auto");
    assert_eq!(app.status, "1 result · read as English");
}

#[test]
fn tab_chooses_how_the_query_is_read() {
    let library = sandbox();
    let mut app = App::new(&library);
    type_text(&mut app, "mao");
    assert_eq!(headword(&app.rows[0]), "猫", "worked out as pinyin");
    // Tab: Hanzi, Pinyin, English, then back to working it out
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.kind, Some(QueryKind::Chinese));
    assert_eq!(app.rows, [], "mao is not characters, so not kept as typed");
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.mode_label(), "Pinyin");
    assert_eq!(headword(&app.rows[0]), "猫");
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.kind, None);
    assert_eq!(app.mode_label(), "Pinyin · auto");
}

#[test]
fn hotkey_letters_always_type_when_query_is_empty() {
    let library = sandbox();
    let mut app = App::new(&library);
    type_text(&mut app, "cepsq");
    assert_eq!(app.query, "cepsq");
    assert!(!app.should_quit);
}

#[test]
fn enter_saves_the_selection_and_marks_it() {
    let library = sandbox();
    let mut app = App::new(&library);
    type_text(&mut app, "school");
    press(&mut app, KeyCode::Enter);
    assert!(
        app.status.starts_with("saved 学校 / 學校 [xué xiào]"),
        "unexpected status: {:?}",
        app.status
    );
    assert_eq!(saved_heads(&library), ["学校"]);
    let Row::Candidate(candidate) = &app.rows[0] else {
        panic!("a dictionary row");
    };
    assert!(candidate.saved.is_some(), "results show it as saved");
    press(&mut app, KeyCode::Enter);
    assert!(
        app.status.starts_with("already saved 学校"),
        "{}",
        app.status
    );
}

#[test]
fn control_z_takes_a_save_back() {
    let library = sandbox();
    let mut app = App::new(&library);
    type_text(&mut app, "school");
    press(&mut app, KeyCode::Enter);
    ctrl(&mut app, KeyCode::Char('z'));
    assert_eq!(app.status, "took back 学校");
    assert_eq!(saved_heads(&library), [] as [String; 0]);
    ctrl(&mut app, KeyCode::Char('z'));
    assert_eq!(app.status, "nothing to take back");
    // Typing moves on: the next save is not taken back by an old Ctrl+Z
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Backspace);
    ctrl(&mut app, KeyCode::Char('z'));
    assert_eq!(saved_heads(&library), ["学校"]);
}

#[test]
fn characters_no_dictionary_has_are_kept_to_fill_in_later() {
    let library = sandbox();
    let mut app = App::new(&library);
    type_text(&mut app, "旅行社");
    assert_eq!(app.rows[0], Row::AsTyped("旅行社".to_owned()));
    assert!(
        app.selected_detail()
            .contains("No dictionary has this word")
    );
    press(&mut app, KeyCode::Enter);
    assert!(
        app.status.starts_with("saved 旅行社 to fill in later"),
        "{}",
        app.status
    );
    let saved = library
        .list_items(&LibraryFilter::default())
        .unwrap()
        .remove(0);
    assert_eq!(saved.simplified, "旅行社");
    assert_eq!(saved.verification, Verification::NeedsReview);
}

#[test]
fn enter_with_no_results_reports_nothing_to_save() {
    let library = sandbox();
    let mut app = App::new(&library);
    type_text(&mut app, "zzzqqq");
    assert_eq!(app.rows, []);
    press(&mut app, KeyCode::Enter);
    assert!(app.status.starts_with("nothing to save"), "{}", app.status);
    assert!(!app.should_quit);
    assert_eq!(saved_heads(&library), [] as [String; 0]);
}

#[test]
fn enter_on_empty_query_is_silent() {
    let library = sandbox();
    let mut app = App::new(&library);
    press(&mut app, KeyCode::Enter);
    assert!(!app.should_quit);
    assert_eq!(app.status, "");
    assert_eq!(saved_heads(&library), [] as [String; 0]);
}

#[test]
fn control_s_does_not_save() {
    let library = sandbox();
    let mut app = App::new(&library);
    type_text(&mut app, "school");
    assert_ne!(app.rows, []);
    ctrl(&mut app, KeyCode::Char('s'));
    assert_eq!(saved_heads(&library), [] as [String; 0]);
    assert!(!app.status.starts_with("saved "), "{:?}", app.status);
}

#[test]
fn escape_clears_query_results_then_quits() {
    let library = sandbox();
    let mut app = App::new(&library);
    type_text(&mut app, "ni");
    assert!(!app.should_quit);
    press(&mut app, KeyCode::Esc);
    assert!(app.query.is_empty(), "Esc clears a typed query");
    assert!(app.rows.is_empty(), "no stale results left behind");
    assert!(!app.should_quit);
    press(&mut app, KeyCode::Esc);
    assert!(app.should_quit, "Esc with an empty query quits");
}

#[test]
fn control_c_quits_regardless_of_query() {
    let library = sandbox();
    let mut app = App::new(&library);
    type_text(&mut app, "ni");
    ctrl(&mut app, KeyCode::Char('c'));
    assert!(app.should_quit);
    assert_eq!(app.query, "ni", "Ctrl+C leaves the typed query intact");
}

#[test]
fn key_release_events_are_ignored() {
    let library = sandbox();
    let mut app = App::new(&library);
    let mut release = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE);
    release.kind = KeyEventKind::Release;
    app.on_key(&release).unwrap();
    assert!(!app.should_quit);
    assert_eq!(app.query, "");
}

#[test]
fn up_down_wrap_through_results() {
    let library = sandbox();
    let mut app = App::new(&library);
    type_text(&mut app, "cat");
    assert!(app.rows.len() >= 2);
    // Live search may restore a non-zero selection while typing, so
    // assert movement relative to wherever typing left the highlight.
    let len = app.rows.len();
    let start = app.list_state.selected().unwrap();
    press(&mut app, KeyCode::Down);
    assert_eq!(app.list_state.selected(), Some((start + 1) % len));
    press(&mut app, KeyCode::Down);
    assert_eq!(app.list_state.selected(), Some((start + 2) % len));
    press(&mut app, KeyCode::Up);
    assert_eq!(app.list_state.selected(), Some((start + 1) % len));
}

#[test]
fn research_restores_selection_by_identity() {
    let library = sandbox();
    let mut app = App::new(&library);
    type_text(&mut app, "cat");
    press(&mut app, KeyCode::Down);
    let selected = headword(app.selected_row().unwrap()).to_owned();
    // A trailing space trims to the same query and searches again.
    press(&mut app, KeyCode::Char(' '));
    press(&mut app, KeyCode::Backspace);
    assert_eq!(headword(app.selected_row().unwrap()), selected);
}

#[test]
fn detail_scroll_moves_and_resets_on_selection_change() {
    let library = sandbox();
    let mut app = App::new(&library);
    type_text(&mut app, "cat");
    assert_eq!(app.detail_scroll, 0);
    press(&mut app, KeyCode::PageDown);
    assert_eq!(app.detail_scroll, crate::ui::DETAIL_SCROLL_STEP);
    press(&mut app, KeyCode::PageUp);
    assert_eq!(app.detail_scroll, 0);
    press(&mut app, KeyCode::PageDown);
    press(&mut app, KeyCode::Down);
    assert_eq!(app.detail_scroll, 0, "moving selection resets the scroll");
}

#[test]
fn typing_clears_a_stale_error() {
    let library = sandbox();
    let mut app = App::new(&library);
    app.error = Some(String::from("boom"));
    press(&mut app, KeyCode::Char('x'));
    assert_eq!(app.error, None);
}

#[test]
fn search_without_a_dictionary_says_why() {
    let library = shouci_core::testing::empty_sandbox();
    let mut app = App::new(&library);
    press(&mut app, KeyCode::Char('x'));
    let error = app.error.clone().expect("an error");
    assert!(error.contains("not loaded"), "{error}");
}

// --- Saved -----------------------------------------------------------

#[test]
fn f1_shows_saved_words_newest_first() {
    let library = sandbox();
    let mut app = App::new(&library);
    search_and_save(&mut app, "school");
    search_and_save(&mut app, "cat");

    press(&mut app, KeyCode::F(1));
    assert_eq!(app.view, View::Saved);
    assert_eq!(app.saved.len(), 2);
    assert!(app.saved[0].id > app.saved[1].id, "newest first");
    assert_eq!(app.saved_state.selected(), Some(0));
    let detail = app.saved_detail();
    assert!(
        detail.contains(&app.saved[0].definition_display),
        "detail shows the definition: {detail}"
    );
    assert!(
        !detail.contains("needs_review"),
        "no machine names: {detail}"
    );
}

#[test]
fn tab_cycles_the_saved_lists() {
    let library = sandbox();
    let mut app = App::new(&library);
    search_and_save(&mut app, "school");
    press(&mut app, KeyCode::F(1));
    assert_eq!(app.scope, Scope::All);
    assert_eq!(app.saved.len(), 1);
    for (scope, len) in [
        (Scope::NeedsReview, 0),
        (Scope::Archived, 0),
        (Scope::Trash, 0),
        (Scope::All, 1),
    ] {
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.scope, scope);
        assert_eq!(app.saved.len(), len, "{scope:?}");
    }
}

#[test]
fn d_moves_to_the_trash_and_r_brings_back() {
    let library = sandbox();
    let mut app = App::new(&library);
    search_and_save(&mut app, "school");
    press(&mut app, KeyCode::F(1));
    press(&mut app, KeyCode::Char('d'));
    assert_eq!(app.saved, []);
    assert_eq!(app.status, "moved 学校 to the trash");
    // Over in the trash, r restores
    for _ in 0..3 {
        press(&mut app, KeyCode::Tab);
    }
    assert_eq!(app.scope, Scope::Trash);
    assert_eq!(app.saved.len(), 1);
    press(&mut app, KeyCode::Char('r'));
    assert_eq!(app.saved, []);
    assert_eq!(saved_heads(&library), ["学校"]);
}

#[test]
fn deleting_from_the_trash_asks_first() {
    let library = sandbox();
    let mut app = App::new(&library);
    search_and_save(&mut app, "school");
    press(&mut app, KeyCode::F(1));
    press(&mut app, KeyCode::Char('d'));
    for _ in 0..3 {
        press(&mut app, KeyCode::Tab);
    }
    press(&mut app, KeyCode::Char('d'));
    assert_eq!(app.saved.len(), 1, "d alone only asks");
    assert!(app.status.contains("y deletes"), "{}", app.status);
    // Any other key keeps it
    press(&mut app, KeyCode::Down);
    assert!(app.status.starts_with("kept "), "{}", app.status);
    press(&mut app, KeyCode::Char('y'));
    assert_eq!(app.saved.len(), 1, "a later y does nothing");
    // y deletes
    press(&mut app, KeyCode::Char('d'));
    press(&mut app, KeyCode::Char('y'));
    assert_eq!(app.saved, []);
    assert_eq!(app.status, "deleted 学校 for good");
    assert!(app.error.is_none(), "{:?}", app.error);
}

#[test]
fn a_archives_and_n_marks_for_review() {
    let library = sandbox();
    let mut app = App::new(&library);
    search_and_save(&mut app, "school");
    search_and_save(&mut app, "cat");
    press(&mut app, KeyCode::F(1));
    press(&mut app, KeyCode::Char('n'));
    assert_eq!(app.saved[0].verification, Verification::NeedsReview);
    assert!(app.status.starts_with("marked "), "{}", app.status);
    press(&mut app, KeyCode::Char('a'));
    assert_eq!(app.saved.len(), 1, "archived words leave the list");
    assert_eq!(
        app.saved_state.selected(),
        Some(0),
        "the selection stays in place"
    );
    press(&mut app, KeyCode::Tab);
    assert_eq!(
        app.saved.len(),
        0,
        "the archived word no longer needs review here"
    );
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.saved.len(), 1);
    press(&mut app, KeyCode::Char('a'));
    assert!(app.status.starts_with("unarchived "), "{}", app.status);
}

#[test]
fn escape_and_f2_return_to_search() {
    let library = sandbox();
    let mut app = App::new(&library);
    press(&mut app, KeyCode::F(1));
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.view, View::Search);
    press(&mut app, KeyCode::F(1));
    press(&mut app, KeyCode::F(2));
    assert_eq!(app.view, View::Search);
}

#[test]
fn saved_reload_preserves_selection_by_id() {
    let library = sandbox();
    let mut app = App::new(&library);
    search_and_save(&mut app, "school");
    search_and_save(&mut app, "cat");
    press(&mut app, KeyCode::F(1));
    press(&mut app, KeyCode::Down);
    let selected_id = app.selected_saved().unwrap().id;
    ctrl(&mut app, KeyCode::Char('r'));
    assert_eq!(app.selected_saved().unwrap().id, selected_id);
}

#[test]
fn shift_tab_toggles_between_views() {
    let library = sandbox();
    let mut app = App::new(&library);
    assert_eq!(app.view, View::Search);
    press(&mut app, KeyCode::BackTab);
    assert_eq!(app.view, View::Saved);
    press(&mut app, KeyCode::BackTab);
    assert_eq!(app.view, View::Search);
}

// --- Import and export -------------------------------------------------

fn open_export_prompt(app: &mut App<'_>) {
    ctrl(app, KeyCode::Char('e'));
}

#[test]
fn export_prompt_starts_on_a_default_destination() {
    let library = sandbox();
    let mut app = App::new(&library);
    open_export_prompt(&mut app);
    let prompt = app.prompt.as_ref().expect("prompt open");
    assert!(prompt.path.ends_with("shouci-pleco.txt"), "{}", prompt.path);
    assert!(Path::new(&prompt.path).is_absolute(), "{}", prompt.path);
    assert!(!prompt.typing, "should offer the panel, not typing");
    assert!(
        app.status.contains("Enter choose location"),
        "status: {:?}",
        app.status
    );
    assert!(app.status.contains("new words only"), "{:?}", app.status);
}

#[test]
fn the_format_toggle_renames_the_default_file() {
    let library = sandbox();
    let mut app = App::new(&library);
    open_export_prompt(&mut app);
    ctrl(&mut app, KeyCode::Char('t'));
    let prompt = app.prompt.as_ref().expect("prompt open");
    assert!(prompt.path.ends_with("shouci-anki.txt"), "{}", prompt.path);
    assert!(app.status.contains("export to Anki"), "{:?}", app.status);
    ctrl(&mut app, KeyCode::Char('t'));
    assert!(app.status.contains("export to Pleco"), "{:?}", app.status);
}

#[test]
fn only_new_and_policy_toggles_belong_to_their_direction() {
    let library = sandbox();
    let mut app = App::new(&library);
    open_export_prompt(&mut app);
    ctrl(&mut app, KeyCode::Char('n'));
    assert!(!app.prompt.as_ref().expect("prompt").only_new);
    assert!(app.status.contains("every word"), "{:?}", app.status);
    ctrl(&mut app, KeyCode::Char('p'));
    assert!(!app.status.contains("words you have"), "{:?}", app.status);

    press(&mut app, KeyCode::Esc);
    ctrl(&mut app, KeyCode::Char('o'));
    assert!(
        app.status.contains("words you have: merge"),
        "the core's default: {:?}",
        app.status
    );
    ctrl(&mut app, KeyCode::Char('p'));
    assert!(
        app.status.contains("words you have: overwrite"),
        "{:?}",
        app.status
    );
}

#[test]
fn enter_requests_the_panel_instead_of_writing() {
    let library = sandbox();
    let mut app = App::new(&library);
    search_and_save(&mut app, "school");
    open_export_prompt(&mut app);
    press(&mut app, KeyCode::Enter);
    let request = app.take_picker().expect("panel requested");
    assert_eq!(request.kind, PromptKind::Export);
    assert!(app.take_picker().is_none(), "request is taken once");
    // Nothing was written: the transfer only runs once a path comes back.
    assert!(!app.status.contains("wrote"), "{:?}", app.status);
}

#[test]
fn import_prompt_offers_the_open_panel() {
    let library = sandbox();
    let mut app = App::new(&library);
    ctrl(&mut app, KeyCode::Char('o'));
    press(&mut app, KeyCode::Enter);
    let request = app.take_picker().expect("panel requested");
    assert_eq!(request.kind, PromptKind::Import);
}

#[test]
fn the_confirm_step_ignores_typed_characters() {
    let library = sandbox();
    let mut app = App::new(&library);
    open_export_prompt(&mut app);
    let before = app.prompt.as_ref().expect("prompt").path.clone();
    type_text(&mut app, "xyz");
    press(&mut app, KeyCode::Backspace);
    assert_eq!(app.prompt.as_ref().expect("prompt").path, before);
    assert!(app.take_picker().is_none(), "no panel without Enter");
}

#[test]
fn a_failed_panel_falls_back_to_typing() {
    let library = sandbox();
    let mut app = App::new(&library);
    open_export_prompt(&mut app);
    app.fall_back_to_typing("no panel available");
    let prompt = app.prompt.as_ref().expect("prompt still open");
    assert!(prompt.typing, "typing enabled after failure");
    assert!(app.status.contains("type the path"), "{:?}", app.status);
    type_text(&mut app, "/tmp");
    assert!(
        app.prompt.as_ref().expect("prompt").path.ends_with("/tmp"),
        "typed into the path"
    );
}

#[test]
fn a_dismissed_panel_leaves_the_prompt_clean() {
    let library = sandbox();
    let mut app = App::new(&library);
    open_export_prompt(&mut app);
    press(&mut app, KeyCode::Enter);
    assert!(app.take_picker().is_some(), "panel requested");
    app.cancel_picker();
    assert!(app.prompt.is_none(), "prompt closed");
    assert_eq!(app.status, "export cancelled — no file written");
}

#[test]
fn tilde_is_expanded_in_the_confirm_step() {
    let library = sandbox();
    let mut app = App::new(&library);
    open_export_prompt(&mut app);
    let home = expand_tilde(Path::new("~")).expect("home");
    app.prompt.as_mut().expect("prompt").typing = true;
    app.prompt.as_mut().expect("prompt").path = String::from("~/definitely-not-here.txt");
    app.refresh_prompt_status();
    assert!(
        app.status.contains(&home.to_string_lossy().into_owned()),
        "status should show the expanded path: {:?}",
        app.status
    );
}

#[test]
fn a_picked_file_is_imported_and_new_words_exported() {
    let library = sandbox();
    let mut app = App::new(&library);
    let dir = scratch_dir("tui-transfer");
    let source = dir.join("from-pleco.txt");
    std::fs::write(&source, "你好\tni3 hao3\thello\n").unwrap();

    ctrl(&mut app, KeyCode::Char('o'));
    app.accept_picked_path(source.to_string_lossy().into_owned());
    assert!(app.error.is_none(), "{:?}", app.error);
    assert!(
        app.status.starts_with("added 1 word from Pleco file"),
        "{}",
        app.status
    );
    assert!(app.prompt.is_none(), "the prompt closes");

    search_and_save(&mut app, "school");
    let out = dir.join("to-pleco.txt");
    open_export_prompt(&mut app);
    app.accept_picked_path(out.to_string_lossy().into_owned());
    assert!(app.status.starts_with("wrote 1 word to "), "{}", app.status);
    assert!(
        app.status.contains("left out 1 word already in Pleco"),
        "{}",
        app.status
    );
    assert!(std::fs::read_to_string(&out).unwrap().contains("学校"));

    open_export_prompt(&mut app);
    app.accept_picked_path(out.to_string_lossy().into_owned());
    assert!(
        app.status.starts_with("nothing to export"),
        "{}",
        app.status
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn pane_hints_advertise_the_main_keys() {
    assert_eq!(
        crate::ui::pane_hint(View::Search, Scope::All),
        "Ctrl+O import · Ctrl+E export"
    );
    assert_eq!(
        crate::ui::pane_hint(View::Saved, Scope::All),
        "i import · e export"
    );
    assert_eq!(
        crate::ui::pane_hint(View::Saved, Scope::Trash),
        "r restore · d delete for good"
    );
}

// --- Drawing -----------------------------------------------------------

fn screen(app: &mut App<'_>, width: u16, height: u16) -> (Vec<String>, ratatui::layout::Position) {
    use ratatui::backend::TestBackend;

    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| app.draw(frame, frame.area()))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let rows = (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol().to_owned())
                .collect()
        })
        .collect();
    (rows, terminal.get_cursor_position().unwrap())
}

#[test]
fn draw_renders_header_and_footer() {
    let library = sandbox();
    let mut app = App::new(&library);
    type_text(&mut app, "ni");
    let (rows, _) = screen(&mut app, 80, 24);
    assert!(rows[2].contains("shouci"), "header shows brand");
    assert!(rows[2].contains("ni"), "header shows query");
    assert!(rows[22].contains("Enter"), "footer shows save hint");
}

#[test]
fn cursor_follows_the_query_for_input_methods() {
    let library = sandbox();
    let mut app = App::new(&library);
    type_text(&mut app, "翻译");
    assert_eq!(app.mode_label(), "Hanzi · auto");
    let (_, cursor) = screen(&mut app, 80, 24);
    // Header box at (1, 1); "shouci [Hanzi · auto]  翻译" is 27 cells
    // wide (Chinese is double width), so the cursor sits at column 2 + 27.
    assert_eq!(cursor, ratatui::layout::Position::new(29, 2));
}

#[test]
fn draw_narrow_terminal_shows_resize_notice() {
    let library = sandbox();
    let mut app = App::new(&library);
    let (rows, _) = screen(&mut app, 40, 10);
    assert!(
        rows.concat().contains("too small"),
        "narrow viewport shows notice"
    );
    // Input still works while the notice is up.
    press(&mut app, KeyCode::Char('a'));
    assert_eq!(app.query, "a");
}
