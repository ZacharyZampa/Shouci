//! Native quick-capture window: `NSPopover` + field + rows + detail.
//!
//! One `define_class!` delegate (`CaptureDelegate`) serves as the field
//! delegate and the click-action target. Results render as plain `NSView`
//! rows (headline + gloss + save tag) with an invisible `NSButton` overlay
//! per row for click selection — deliberately no `NSTableView`, whose
//! data-source methods return retained objects, which `define_class!`
//! methods cannot express.
//!
//! All state lives in ivars as `RefCell`s; every method runs on the main
//! thread by construction (`MainThreadOnly`).
//!
//! Borrowing discipline: `RefCell` borrows are never held across an `AppKit`
//! call that can reenter the delegate. Each borrow is a tight scope around
//! plain-data access.
//!
//! Native behaviors used instead of recreated: `Transient` popover
//! light-dismiss, field-editor key commands, button target/action,
//! `NSTimer` debounce, `NSAnimationContext` fades, `CASpringAnimation`.

use std::cell::RefCell;
use std::path::Path;
use std::sync::mpsc;
use std::time::SystemTime;

use objc2::ffi::NSInteger;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAccessibility, NSAccessibilityAnnouncementKey,
    NSAccessibilityAnnouncementRequestedNotification, NSAccessibilityPostNotificationWithUserInfo,
    NSAlert, NSAlertSecondButtonReturn, NSAnimatablePropertyContainer, NSAnimationContext,
    NSApplication, NSBackingStoreType, NSBox, NSBoxType, NSButton, NSButtonType, NSColor,
    NSControl, NSControlStateValueOff, NSControlStateValueOn, NSControlTextEditingDelegate,
    NSEventModifierFlags, NSEventType, NSFont, NSImage, NSImageView, NSLayoutAttribute,
    NSLayoutConstraint, NSLayoutConstraintOrientation, NSLineBreakMode, NSMenu, NSMenuItem,
    NSPopover, NSPopoverBehavior, NSPopoverDelegate, NSScrollView, NSStackView, NSStatusBarButton,
    NSTextAlignment, NSTextField, NSTextFieldDelegate, NSTextView, NSTitlePosition,
    NSUserInterfaceLayoutOrientation, NSView, NSViewController, NSWindow, NSWindowStyleMask,
    NSWorkspace,
};
use objc2_foundation::{
    NSDictionary, NSNotification, NSNumber, NSPoint, NSRect, NSRectEdge, NSSize, NSString, NSTimer,
    ns_string,
};
use objc2_quartz_core::{CABasicAnimation, CAMediaTiming, CASpringAnimation};
use vocab_capture::{save_candidate, search_auto};
use vocab_core::ItemStatus;
use vocab_db::{SaveOutcome, delete_item, find_saved_item, find_saved_reading, set_status};
use vocab_dictionary::{Candidate, SqliteDictionary};
use vocab_search::{DeterministicRanker, SearchService};

use crate::capture::RESULT_LIMIT;
use crate::debuglog;
use crate::layout::{FlippedView, activate, add_full_width, column, manual, pin_edges};

const DEBOUNCE_INTERVAL: f64 = 0.15;
const DISMISS_HOLD: f64 = 1.5;
const FADE_DURATION: f64 = 0.25;
const FADE_OUT_DURATION: f64 = 0.2;
/// Popover width. Height is not a constant: it follows the content.
const CONTENT_WIDTH: f64 = 400.0;
/// Margin around the popover content.
const INSET: f64 = 12.0;
/// Width available to text inside the margins.
const TEXT_WIDTH: f64 = CONTENT_WIDTH - 2.0 * INSET;
/// The list grows with its results up to this many rows, then scrolls. The
/// half row shows at a glance that there is more below.
const VISIBLE_ROWS: f64 = 5.5;
/// Text width in the Settings window (its content is 20 pt inset).
const SETTINGS_TEXT_WIDTH: f64 = 360.0;
/// How often the main thread checks on a background dictionary fetch.
const DICTIONARY_POLL_INTERVAL: f64 = 0.5;

/// Where the search dictionary stands. The popover opens in every state;
/// only `Ready` searches.
enum Dictionary {
    /// First-run download in progress; no file to search yet.
    Loading,
    Ready(SearchService<SqliteDictionary>),
    Failed(String),
}

/// A background `ensure_dictionary_db` run (first download or monthly
/// refresh), polled from the main thread by an `NSTimer`.
struct DictionaryJob {
    done: mpsc::Receiver<Result<(), String>>,
    poll: Retained<NSTimer>,
    /// File modification time before the run. A change means a refresh
    /// replaced the file, so the open service must be reopened.
    before: Option<SystemTime>,
}

fn open_dictionary(path: &Path) -> Result<SearchService<SqliteDictionary>, String> {
    let provider = SqliteDictionary::open(path).map_err(|err| err.to_string())?;
    Ok(SearchService::new(provider, DeterministicRanker::default()))
}

fn modified_at(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

/// Status line after a search: the count, plus how the query was read when
/// it wasn't the obvious guess (typing `jingzi` and getting pinyin hits).
fn results_summary(count: usize, mode: &str) -> String {
    let results = if count == 1 {
        String::from("1 result")
    } else {
        format!("{count} results")
    };
    if mode == "auto" {
        results
    } else {
        format!("{results} · read as {mode}")
    }
}

/// Whether the row at `index` is already in the user's list.
fn rows_saved(rows: &[RowViews], index: usize) -> bool {
    rows.get(index).is_some_and(|row| row.saved.is_some())
}

/// Spoken and tooltip description of a row: headword, reading, gloss, and
/// whether it is already in the user's list.
fn row_description(hit: &Candidate, saved: bool) -> String {
    let mut text = format!(
        "{}: {}",
        crate::capture::headline(hit),
        crate::capture::gloss_line(hit)
    );
    if saved {
        text.push_str(" (already saved)");
    }
    text
}

/// Handles to one result row's live views, plus the saved state behind the
/// badge (`None` = not in the user database).
struct RowViews {
    view: Retained<NSView>,
    /// The row's click target, which is also its accessibility element.
    click: Retained<NSButton>,
    /// System selection fill behind the text, shown on the selected row.
    highlight: Retained<NSBox>,
    headline: Retained<NSTextField>,
    reading: Retained<NSTextField>,
    gloss: Retained<NSTextField>,
    tag: Retained<NSTextField>,
    saved: Option<ItemStatus>,
}

// `pub(crate)` because the `pub(crate)` delegate class names it in
// `#[ivars = …]`; fields stay module-private, accessed via `ivars()`.
pub(crate) struct CaptureIvars {
    anchor: Retained<NSStatusBarButton>,
    popover: Retained<NSPopover>,
    context_menu: Retained<NSMenu>,
    field: Retained<NSTextField>,
    /// The popover's root stack; its fitting size is the popover size.
    content: Retained<NSStackView>,
    scroll: Retained<NSScrollView>,
    /// Result rows, top to bottom, inside the scroll view's document.
    list: Retained<NSStackView>,
    /// Scroll view height: the rows' height, capped at [`VISIBLE_ROWS`].
    list_height: Retained<NSLayoutConstraint>,
    detail: Retained<NSTextField>,
    status: Retained<NSTextField>,
    success: Retained<NSView>,
    check: Retained<NSImageView>,
    success_title: Retained<NSTextField>,
    success_message: Retained<NSTextField>,
    search_timer: RefCell<Option<Retained<NSTimer>>>,
    dismiss_timer: RefCell<Option<Retained<NSTimer>>>,
    hotkey: RefCell<Option<crate::hotkey::Registration>>,
    settings: RefCell<Option<Retained<NSWindow>>>,
    recorder: RefCell<Option<Retained<crate::settings::HotkeyRecorder>>>,
    settings_status: RefCell<Option<Retained<NSTextField>>>,
    login_checkbox: RefCell<Option<Retained<NSButton>>>,
    dictionary: RefCell<Dictionary>,
    dictionary_job: RefCell<Option<DictionaryJob>>,
    user_conn: rusqlite::Connection,
    results: RefCell<Vec<Candidate>>,
    rows: RefCell<Vec<RowViews>>,
    selection: RefCell<usize>,
}

define_class!(
    /// Controller for the quick-capture popover. See module docs.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = CaptureIvars]
    pub(crate) struct CaptureDelegate;

    // SAFETY: `NSObjectProtocol` has no safety requirements.
    unsafe impl NSObjectProtocol for CaptureDelegate {}


    // SAFETY: `NSPopoverDelegate` has no safety requirements. Only the
    // close notification is implemented, purely as a diagnostic: it fires
    // for system-driven closes (light-dismiss) that bypass our methods.
    unsafe impl NSPopoverDelegate for CaptureDelegate {
        #[unsafe(method(popoverDidClose:))]
        fn popover_did_close(&self, _notification: &NSNotification) {
            debuglog::event("popover did-close notification");
        }
    }

    // SAFETY: `NSTextFieldDelegate` has no safety requirements. It is
    // empty: the field's callbacks below are declared by its superprotocol,
    // and objc2 (in debug builds) rejects them on the wrong one at launch.
    unsafe impl NSTextFieldDelegate for CaptureDelegate {}

    // SAFETY: `NSControlTextEditingDelegate` has no safety requirements; the
    // methods only touch main-thread ivars with scoped borrows.
    unsafe impl NSControlTextEditingDelegate for CaptureDelegate {
        #[unsafe(method(controlTextDidChange:))]
        fn control_text_did_change(&self, _obj: &NSNotification) {
            self.schedule_search();
        }

        #[unsafe(method(control:textView:doCommandBySelector:))]
        unsafe fn control_text_view_do_command(
            &self,
            _control: &NSControl,
            _text_view: &NSTextView,
            command: Sel,
        ) -> bool {
            let name = if command == sel!(moveDown:) {
                "moveDown"
            } else if command == sel!(moveUp:) {
                "moveUp"
            } else if command == sel!(insertNewline:) {
                "insertNewline"
            } else if command == sel!(cancelOperation:) {
                "cancelOperation"
            } else {
                "other"
            };
            debuglog::event(&format!("key-command {name}"));
            if command == sel!(moveDown:) {
                self.move_selection(1);
                true
            } else if command == sel!(moveUp:) {
                self.move_selection(-1);
                true
            } else if command == sel!(insertNewline:) {
                self.save_selected();
                true
            } else if command == sel!(cancelOperation:) {
                self.hide();
                true
            } else {
                false
            }
        }
    }

    impl CaptureDelegate {
        /// Row click action. The sender is always one of our overlay buttons;
        /// its tag is the result index.
        #[unsafe(method(selectRow:))]
        fn select_row_action(&self, sender: &NSButton) {
            if let Ok(index) = usize::try_from(sender.tag()) {
                self.select_row(index);
            }
        }

        /// Context-menu Save: acts on the right-clicked row, not the selection.
        #[unsafe(method(saveRowFromMenu:))]
        fn save_row_from_menu(&self, sender: &NSMenuItem) {
            if let Ok(index) = usize::try_from(sender.tag()) {
                self.save_row(index);
            }
        }

        /// Context-menu "Mark as Needs Review".
        #[unsafe(method(markReviewFromMenu:))]
        fn mark_review_from_menu(&self, sender: &NSMenuItem) {
            if let Ok(index) = usize::try_from(sender.tag()) {
                self.set_status_for_row(index, ItemStatus::NeedsReview);
            }
        }

        /// Context-menu "Archive".
        #[unsafe(method(archiveFromMenu:))]
        fn archive_from_menu(&self, sender: &NSMenuItem) {
            if let Ok(index) = usize::try_from(sender.tag()) {
                self.set_status_for_row(index, ItemStatus::Archived);
            }
        }

        /// Context-menu "Delete…" (confirmed; see [`CaptureDelegate::delete_row`]).
        #[unsafe(method(deleteFromMenu:))]
        fn delete_from_menu(&self, sender: &NSMenuItem) {
            if let Ok(index) = usize::try_from(sender.tag()) {
                self.delete_row(index);
            }
        }

        /// Main-menu Word → Mark as Needs Review (⌘R), on the selected row.
        #[unsafe(method(markReviewSelected:))]
        fn mark_review_selected(&self, _sender: Option<&AnyObject>) {
            let index = *self.ivars().selection.borrow();
            self.set_status_for_row(index, ItemStatus::NeedsReview);
        }

        /// Main-menu Word → Archive (⌃⌘A), on the selected row.
        #[unsafe(method(archiveSelected:))]
        fn archive_selected(&self, _sender: Option<&AnyObject>) {
            let index = *self.ivars().selection.borrow();
            self.set_status_for_row(index, ItemStatus::Archived);
        }

        /// Main-menu Word → Delete… (⌥⌘⌫), on the selected row.
        #[unsafe(method(deleteSelected:))]
        fn delete_selected(&self, _sender: Option<&AnyObject>) {
            let index = *self.ivars().selection.borrow();
            self.delete_row(index);
        }

        /// Enables the selection-based Word items only while the popover
        /// shows results, so their shortcuts never act on a hidden row.
        #[unsafe(method(validateMenuItem:))]
        fn validate_menu_item(&self, item: &NSMenuItem) -> bool {
            let on_selection = [
                sel!(markReviewSelected:),
                sel!(archiveSelected:),
                sel!(deleteSelected:),
            ];
            // One tail expression: `define_class!` converts only the method's
            // final value to `Bool`, not an early `return`.
            let ivars = self.ivars();
            !item.action().is_some_and(|action| on_selection.contains(&action))
                || (ivars.popover.isShown()
                    && ivars.success.isHidden()
                    && !ivars.results.borrow().is_empty())
        }

        /// Checks on the background dictionary fetch (repeating timer).
        #[unsafe(method(pollDictionary:))]
        fn poll_dictionary(&self, _timer: &NSTimer) {
            self.finish_dictionary_job_if_done();
        }

        /// Button action for both mouse buttons (see `sendActionOn` wiring in
        /// `main`). Left click toggles the popover; right-click pops the
        /// menu instead. Branching on the event is the native pattern for a
        /// status item with action + menu behavior and no `item.menu`.
        #[unsafe(method(togglePopover:))]
        fn toggle_popover(&self, _sender: Option<&AnyObject>) {
            let right_click = NSApplication::sharedApplication(self.mtm())
                .currentEvent()
                .is_some_and(|event| event.r#type() == NSEventType::RightMouseUp);
            if right_click {
                self.show_context_menu();
            } else {
                self.toggle();
            }
        }

        /// Debounce timer: runs the search on the main thread.
        #[unsafe(method(runDebouncedSearch:))]
        fn run_debounced_search(&self, _timer: &NSTimer) {
            self.run_search_now();
        }

        /// Dismiss timer: fade the success view out, then close.
        #[unsafe(method(dismissAfterHold:))]
        fn dismiss_after_hold(&self, _timer: &NSTimer) {
            NSAnimationContext::beginGrouping();
            NSAnimationContext::currentContext().setDuration(FADE_OUT_DURATION);
            self.ivars().success.animator().setAlphaValue(0.0);
            NSAnimationContext::endGrouping();
            let timer = unsafe {
                NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                    FADE_OUT_DURATION + 0.1,
                    self,
                    sel!(finishFadeOut:),
                    None,
                    false,
                )
            };
            *self.ivars().dismiss_timer.borrow_mut() = Some(timer);
        }

        /// End of the fade-out: close the popover.
        #[unsafe(method(finishFadeOut:))]
        fn finish_fade_out(&self, _timer: &NSTimer) {
            debuglog::event("hide reason=save-dismiss");
            self.ivars().popover.close();
        }

        /// `NSMenuItem` action for "Settings…".
        #[unsafe(method(showSettings:))]
        fn show_settings_action(&self, _sender: Option<&AnyObject>) {
            self.show_settings();
        }

        /// Login checkbox action.
        #[unsafe(method(toggleLoginItem:))]
        fn toggle_login_item(&self, sender: &NSButton) {
            use objc2_app_kit::NSControlStateValueOn;
            let enabled = sender.state() == NSControlStateValueOn;
            match crate::login::set_enabled(enabled) {
                Ok(()) => {
                    debuglog::event(&format!("login-at-startup {enabled}"));
                    self.refresh_login_checkbox();
                    self.settings_status(if enabled {
                        "Shouci will open when you log in."
                    } else {
                        "Shouci won't open when you log in."
                    });
                }
                Err(message) => {
                    debuglog::event(&format!("login error {message}"));
                    // Revert the checkbox: the change did not take effect.
                    self.refresh_login_checkbox();
                    self.settings_error(&format!(
                        "Couldn't turn Open at Login {}: {message}",
                        if enabled { "on" } else { "off" }
                    ));
                }
            }
        }
    }
);

/// `NSString` from Rust text (`ns_string!` only takes literals).
fn ns_string_from(text: &str) -> Retained<NSString> {
    NSString::from_str(text)
}

/// A label that wraps to at most `lines` lines and ends the last one with
/// an ellipsis, so long glosses and errors never clip mid-glyph.
fn wrapping_label(mtm: MainThreadMarker, lines: NSInteger) -> Retained<NSTextField> {
    let label = NSTextField::wrappingLabelWithString(ns_string!(""), mtm);
    label.setMaximumNumberOfLines(lines);
    if let Some(cell) = label.cell() {
        cell.setTruncatesLastVisibleLine(true);
    }
    label
}

/// One-line row text: truncates at the tail instead of clipping.
fn single_line(field: &NSTextField) {
    field.setUsesSingleLineMode(true);
    field.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
}

fn build_field(mtm: MainThreadMarker) -> Retained<NSTextField> {
    let field = NSTextField::new(mtm);
    field.setEditable(true);
    field.setSelectable(true);
    field.setPlaceholderString(Some(ns_string!("pinyin, English, or 中文…")));
    field.setFont(Some(&NSFont::systemFontOfSize(15.0)));
    single_line(&field);
    field
}

/// Scroll view over a flipped document holding a stack of rows. The scroll
/// view starts hidden (no results) and at zero height.
fn build_list(
    mtm: MainThreadMarker,
) -> (
    Retained<NSScrollView>,
    Retained<NSStackView>,
    Retained<NSLayoutConstraint>,
) {
    let scroll = NSScrollView::new(mtm);
    scroll.setHasVerticalScroller(true);
    scroll.setAutohidesScrollers(true);
    let document = FlippedView::new(mtm);
    let list = column(mtm, 0.0, 0.0);
    document.addSubview(&list);
    scroll.setDocumentView(Some(&document));
    let clip = scroll.contentView();
    let list_height = scroll.heightAnchor().constraintEqualToConstant(0.0);
    let mut constraints = pin_edges(&list, &document, 0.0);
    constraints.extend([
        document
            .topAnchor()
            .constraintEqualToAnchor(&clip.topAnchor()),
        document
            .leadingAnchor()
            .constraintEqualToAnchor(&clip.leadingAnchor()),
        document
            .widthAnchor()
            .constraintEqualToAnchor(&clip.widthAnchor()),
        list_height.clone(),
    ]);
    activate(&constraints);
    scroll.setHidden(true);
    (scroll, list, list_height)
}

fn build_labels(mtm: MainThreadMarker) -> (Retained<NSTextField>, Retained<NSTextField>) {
    // The detail shows the whole definition of the selected row, which the
    // one-line row gloss may have cut short.
    let detail = wrapping_label(mtm, 5);
    detail.setPreferredMaxLayoutWidth(TEXT_WIDTH);
    detail.setFont(Some(&NSFont::systemFontOfSize(12.0)));
    detail.setTextColor(Some(&NSColor::secondaryLabelColor()));
    detail.setHidden(true);

    let status = wrapping_label(mtm, 3);
    status.setPreferredMaxLayoutWidth(TEXT_WIDTH);
    status.setFont(Some(&NSFont::systemFontOfSize(11.0)));
    status.setTextColor(Some(&NSColor::secondaryLabelColor()));
    status.setHidden(true);
    (detail, status)
}

fn build_success(
    mtm: MainThreadMarker,
) -> (
    Retained<NSView>,
    Retained<NSImageView>,
    Retained<NSTextField>,
    Retained<NSTextField>,
) {
    let success = NSView::new(mtm);
    success.setWantsLayer(true);
    success.setAlphaValue(0.0);
    success.setHidden(true);
    let check = NSImageView::new(mtm);
    if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        ns_string!("checkmark.circle.fill"),
        Some(ns_string!("saved")),
    ) {
        check.setImage(Some(&image));
    }
    check.setWantsLayer(true);
    let success_title = NSTextField::labelWithString(ns_string!("Saved"), mtm);
    success_title.setFont(Some(&NSFont::boldSystemFontOfSize(15.0)));
    single_line(&success_title);
    let success_message = wrapping_label(mtm, 2);
    success_message.setPreferredMaxLayoutWidth(TEXT_WIDTH - 56.0);
    success_message.setFont(Some(&NSFont::systemFontOfSize(12.0)));
    success_message.setTextColor(Some(&NSColor::secondaryLabelColor()));
    for view in [&*check, &**success_title, &**success_message] {
        manual(view);
        success.addSubview(view);
    }
    // Check on the left; title and message stacked beside it. The view is
    // as tall as the taller of the two, never taller (the low-priority zero
    // height pulls it down onto them).
    let collapse = success.heightAnchor().constraintEqualToConstant(0.0);
    collapse.setPriority(250.0);
    activate(&[
        check
            .topAnchor()
            .constraintEqualToAnchor(&success.topAnchor()),
        check
            .leadingAnchor()
            .constraintEqualToAnchor(&success.leadingAnchor()),
        check.widthAnchor().constraintEqualToConstant(44.0),
        check.heightAnchor().constraintEqualToConstant(44.0),
        success_title
            .leadingAnchor()
            .constraintEqualToAnchor_constant(&check.trailingAnchor(), 12.0),
        success_title
            .topAnchor()
            .constraintEqualToAnchor_constant(&check.topAnchor(), 2.0),
        success_title
            .trailingAnchor()
            .constraintLessThanOrEqualToAnchor(&success.trailingAnchor()),
        success_message
            .leadingAnchor()
            .constraintEqualToAnchor(&success_title.leadingAnchor()),
        success_message
            .topAnchor()
            .constraintEqualToAnchor_constant(&success_title.bottomAnchor(), 2.0),
        success_message
            .trailingAnchor()
            .constraintEqualToAnchor(&success.trailingAnchor()),
        success
            .bottomAnchor()
            .constraintGreaterThanOrEqualToAnchor(&check.bottomAnchor()),
        success
            .bottomAnchor()
            .constraintGreaterThanOrEqualToAnchor(&success_message.bottomAnchor()),
        collapse,
    ]);
    (success, check, success_title, success_message)
}

/// Row type roles. The headword is what the user came to capture, so it
/// leads; the reading sits beside it on the same baseline, quieter; the
/// gloss is supporting detail underneath.
const HEADWORD_SIZE: f64 = 15.0;
const READING_SIZE: f64 = 12.0;
const GLOSS_SIZE: f64 = 11.0;

/// The views of one freshly built row.
struct BuiltRow {
    view: Retained<NSView>,
    highlight: Retained<NSBox>,
    headline: Retained<NSTextField>,
    reading: Retained<NSTextField>,
    tag: Retained<NSTextField>,
    gloss: Retained<NSTextField>,
    click: Retained<NSButton>,
}

/// One result row: headword, reading, and save tag on the first line, the
/// gloss below. Height comes from the text; nothing is positioned by hand.
fn build_row(mtm: MainThreadMarker, headword: &str, reading: &str, gloss_text: &str) -> BuiltRow {
    let view = NSView::new(mtm);
    // The system's list selection: accent-tinted, rounded, and first in the
    // subview order so the text draws on top of it.
    let highlight = NSBox::new(mtm);
    highlight.setBoxType(NSBoxType::Custom);
    highlight.setTitlePosition(NSTitlePosition::NoTitle);
    highlight.setBorderWidth(0.0);
    highlight.setCornerRadius(5.0);
    highlight.setContentViewMargins(NSSize::new(0.0, 0.0));
    highlight.setFillColor(&NSColor::selectedContentBackgroundColor());
    highlight.setHidden(true);
    manual(&highlight);
    view.addSubview(&highlight);
    let headline = NSTextField::labelWithString(&ns_string_from(headword), mtm);
    headline.setFont(Some(&NSFont::systemFontOfSize(HEADWORD_SIZE)));
    single_line(&headline);
    let reading = NSTextField::labelWithString(&ns_string_from(reading), mtm);
    reading.setFont(Some(&NSFont::systemFontOfSize(READING_SIZE)));
    reading.setTextColor(Some(&NSColor::secondaryLabelColor()));
    single_line(&reading);
    let tag = NSTextField::labelWithString(ns_string!("↩ Save"), mtm);
    tag.setFont(Some(&NSFont::boldSystemFontOfSize(GLOSS_SIZE)));
    tag.setAlignment(NSTextAlignment::Right);
    single_line(&tag);
    tag.setHidden(true);
    let gloss = NSTextField::labelWithString(&ns_string_from(gloss_text), mtm);
    gloss.setFont(Some(&NSFont::systemFontOfSize(GLOSS_SIZE)));
    gloss.setTextColor(Some(&NSColor::secondaryLabelColor()));
    single_line(&gloss);
    // Invisible click target over the whole row. Its tag carries the
    // result index to the action method; the button itself draws nothing.
    let click = NSButton::new(mtm);
    click.setTransparent(true);
    click.setBordered(false);
    for label in [&headline, &reading, &tag, &gloss] {
        manual(label);
        view.addSubview(label);
    }
    manual(&click);
    view.addSubview(&click);
    // When space runs out the reading truncates first, then the headword;
    // the tag never gives way.
    headline.setContentCompressionResistancePriority_forOrientation(
        500.0,
        NSLayoutConstraintOrientation::Horizontal,
    );
    reading.setContentCompressionResistancePriority_forOrientation(
        250.0,
        NSLayoutConstraintOrientation::Horizontal,
    );
    let mut constraints = pin_edges(&click, &view, 0.0);
    constraints.extend(pin_edges(&highlight, &view, 0.0));
    constraints.extend([
        headline
            .topAnchor()
            .constraintEqualToAnchor_constant(&view.topAnchor(), 5.0),
        headline
            .leadingAnchor()
            .constraintEqualToAnchor_constant(&view.leadingAnchor(), 8.0),
        reading
            .leadingAnchor()
            .constraintEqualToAnchor_constant(&headline.trailingAnchor(), 8.0),
        reading
            .firstBaselineAnchor()
            .constraintEqualToAnchor(&headline.firstBaselineAnchor()),
        reading
            .trailingAnchor()
            .constraintLessThanOrEqualToAnchor_constant(&tag.leadingAnchor(), -8.0),
        // Fixed tag width: the truncation point does not jump as the
        // selection (and so the tag text) moves.
        tag.widthAnchor().constraintEqualToConstant(56.0),
        tag.trailingAnchor()
            .constraintEqualToAnchor_constant(&view.trailingAnchor(), -8.0),
        tag.firstBaselineAnchor()
            .constraintEqualToAnchor(&headline.firstBaselineAnchor()),
        gloss
            .topAnchor()
            .constraintEqualToAnchor_constant(&headline.bottomAnchor(), 1.0),
        gloss
            .leadingAnchor()
            .constraintEqualToAnchor(&headline.leadingAnchor()),
        gloss
            .trailingAnchor()
            .constraintEqualToAnchor_constant(&view.trailingAnchor(), -8.0),
        gloss
            .bottomAnchor()
            .constraintEqualToAnchor_constant(&view.bottomAnchor(), -6.0),
    ]);
    activate(&constraints);
    BuiltRow {
        view,
        highlight,
        headline,
        reading,
        tag,
        gloss,
        click,
    }
}

impl CaptureDelegate {
    fn new(
        mtm: MainThreadMarker,
        anchor: Retained<NSStatusBarButton>,
        user_conn: rusqlite::Connection,
    ) -> Retained<Self> {
        let popover = NSPopover::new(mtm);
        popover.setBehavior(NSPopoverBehavior::Transient);

        // SAFETY: allocating views then running the designated initializer
        // before any other use; all objects stay owned by ivars/views.
        unsafe {
            // Field, results, detail, save confirmation, status: top to
            // bottom. Empty sections are hidden and drop out, so a fresh
            // popover is just the field and grows as results arrive.
            let content = column(mtm, 8.0, INSET);
            activate(&[content
                .widthAnchor()
                .constraintEqualToConstant(CONTENT_WIDTH)]);
            let field = build_field(mtm);
            let (scroll, list, list_height) = build_list(mtm);
            let (detail, status) = build_labels(mtm);
            let (success, check, success_title, success_message) = build_success(mtm);
            for view in [&*field, &**scroll, &**detail, &*success, &**status] {
                add_full_width(&content, view, INSET);
            }
            content.setCustomSpacing_afterView(10.0, &field);

            let controller = NSViewController::new(mtm);
            controller.setView(&content);
            popover.setContentViewController(Some(&controller));

            // Right-click menu (Settings…, Quit). Left-click never sees it:
            // the button action below fires on both buttons and branches on
            // the event, so `item.menu` stays unset on purpose.
            let context_menu = NSMenu::new(mtm);
            // SAFETY: standard initializers; items retained by the menu.
            let settings_item = NSMenuItem::initWithTitle_action_keyEquivalent(
                mtm.alloc(),
                ns_string!("Settings…"),
                Some(sel!(showSettings:)),
                ns_string!(","),
            );
            let quit_item = NSMenuItem::initWithTitle_action_keyEquivalent(
                mtm.alloc(),
                ns_string!("Quit Shouci"),
                Some(sel!(terminate:)),
                ns_string!("q"),
            );
            context_menu.addItem(&settings_item);
            context_menu.addItem(&quit_item);

            let this = Self::alloc(mtm).set_ivars(CaptureIvars {
                anchor,
                popover,
                context_menu,
                field,
                content,
                scroll,
                list,
                list_height,
                detail,
                status,
                success,
                check,
                success_title,
                success_message,
                search_timer: RefCell::new(None),
                dismiss_timer: RefCell::new(None),
                hotkey: RefCell::new(None),
                settings: RefCell::new(None),
                recorder: RefCell::new(None),
                settings_status: RefCell::new(None),
                login_checkbox: RefCell::new(None),
                dictionary: RefCell::new(Dictionary::Loading),
                dictionary_job: RefCell::new(None),
                user_conn,
                results: RefCell::new(Vec::new()),
                rows: RefCell::new(Vec::new()),
                selection: RefCell::new(0),
            });
            // SAFETY: `NSObject init` on freshly allocated storage with all
            // ivars in place; matches the documented `define_class!` pattern.
            let this: Retained<Self> = msg_send![super(this), init];
            // Delegate wiring needs `&self` as an object, so it happens
            // here. SAFETY: weak properties; `this` outlives the views
            // (process lifetime via the shared slot below).
            this.ivars()
                .field
                .setDelegate(Some(ProtocolObject::from_ref(&*this)));
            this.ivars()
                .popover
                .setDelegate(Some(ProtocolObject::from_ref(&*this)));
            // Menu actions target the delegate, except Quit: `terminate:`
            // with no target travels the responder chain to `NSApplication`.
            // SAFETY: the delegate outlives the menu (process lifetime).
            settings_item.setTarget(Some(&*this));
            this
        }
    }

    /// Shows the popover under the status button, or hides it if open.
    pub fn toggle(&self) {
        let ivars = self.ivars();
        if ivars.popover.isShown() {
            debuglog::event("hide reason=toggle");
            ivars.popover.close();
        } else {
            self.show();
        }
    }

    fn show_context_menu(&self) {
        let ivars = self.ivars();
        if let Some(event) = NSApplication::sharedApplication(self.mtm()).currentEvent() {
            NSMenu::popUpContextMenu_withEvent_forView(&ivars.context_menu, &event, &ivars.anchor);
        }
    }

    /// Registers the stored (or default) hotkey. Called once at startup;
    /// rebinds go through [`CaptureDelegate::rebind_hotkey`].
    pub(crate) fn bind_default_hotkey(&self) {
        let binding = crate::settings::load_binding();
        // SAFETY: main run loop is pumping (`NSApplication::run`).
        match unsafe {
            crate::hotkey::register(binding.keycode, binding.modifiers, || {
                toggle_shared();
            })
        } {
            Ok(registration) => {
                *self.ivars().hotkey.borrow_mut() = Some(registration);
            }
            Err(err) => debuglog::event(&format!("hotkey error {err}")),
        }
    }

    /// Drops the current registration and registers `binding`. Reports
    /// failures to the settings status line instead of failing silently.
    fn rebind_hotkey(&self, binding: crate::settings::HotkeyBinding) {
        drop(self.ivars().hotkey.borrow_mut().take());
        // SAFETY: same contract as startup registration.
        match unsafe {
            crate::hotkey::register(binding.keycode, binding.modifiers, || {
                toggle_shared();
            })
        } {
            Ok(registration) => {
                *self.ivars().hotkey.borrow_mut() = Some(registration);
                let shortcut = crate::settings::display_string();
                self.settings_status(&format!(
                    "Quick Capture is now {shortcut}. It works in any app."
                ));
                self.ivars()
                    .anchor
                    .setToolTip(Some(&ns_string_from(&format!(
                        "Shouci — {shortcut} to capture a word"
                    ))));
            }
            Err(err) => {
                debuglog::event(&format!("hotkey error {err}"));
                // The previous binding was dropped above, so say so plainly.
                self.settings_error(
                    "Couldn't use that shortcut; another app may already use it. \
                     Choose a different one. Until then, click 文 in the menu bar to capture.",
                );
            }
        }
    }

    fn show_settings(&self) {
        let ivars = self.ivars();
        if let Some(window) = ivars.settings.borrow().as_ref() {
            window.makeKeyAndOrderFront(None);
            NSApplication::sharedApplication(self.mtm()).activate();
            return;
        }
        let mtm = self.mtm();
        // SAFETY: designated initializer on a fresh allocation; the window
        // is retained in ivars below.
        let window: Retained<NSWindow> = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(400.0, 250.0)),
                NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        window.setTitle(ns_string!("Shouci Settings"));
        // SAFETY: we retain the window in ivars, so AppKit must not release
        // it on close (same contract as the hello-world example).
        unsafe {
            window.setReleasedWhenClosed(false);
        }
        let content = window.contentView().expect("window must have content view");
        // Hotkey row, login switch, then status: sized by their content, so
        // the window fits them instead of the other way round.
        let form = column(mtm, 14.0, 20.0);
        content.addSubview(&form);
        activate(&pin_edges(&form, &content, 0.0));

        let hotkey_label = NSTextField::labelWithString(ns_string!("Quick Capture hotkey:"), mtm);
        hotkey_label.setFont(Some(&NSFont::systemFontOfSize(13.0)));
        let recorder = crate::settings::recorder_view(mtm);
        let hotkey_row = NSStackView::new(mtm);
        hotkey_row.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
        hotkey_row.setAlignment(NSLayoutAttribute::CenterY);
        hotkey_row.setSpacing(12.0);
        hotkey_row.addArrangedSubview(&hotkey_label);
        hotkey_row.addArrangedSubview(&recorder);
        manual(&hotkey_row);
        form.addArrangedSubview(&hotkey_row);

        let login = NSButton::new(mtm);
        login.setButtonType(NSButtonType::Switch);
        login.setTitle(ns_string!("Open at Login"));
        login.setFont(Some(&NSFont::systemFontOfSize(13.0)));
        // SAFETY: target/action; the delegate outlives the window.
        unsafe {
            login.setTarget(Some(self));
            login.setAction(Some(sel!(toggleLoginItem:)));
        }
        manual(&login);
        form.addArrangedSubview(&login);

        let status = wrapping_label(mtm, 0);
        status.setPreferredMaxLayoutWidth(SETTINGS_TEXT_WIDTH);
        status.setFont(Some(&NSFont::systemFontOfSize(11.0)));
        status.setTextColor(Some(&NSColor::secondaryLabelColor()));
        status.setHidden(true);
        add_full_width(&form, &status, 20.0);
        activate(&[form
            .widthAnchor()
            .constraintGreaterThanOrEqualToConstant(SETTINGS_TEXT_WIDTH + 40.0)]);

        *ivars.recorder.borrow_mut() = Some(recorder);
        *ivars.settings_status.borrow_mut() = Some(status);
        *ivars.login_checkbox.borrow_mut() = Some(login);
        *ivars.settings.borrow_mut() = Some(window.clone());
        // `ivars` is a shared borrow; it ends here by NLL before the calls
        // below re-borrow through `self`.
        self.refresh_login_checkbox();
        // Tab moves between the recorder and the checkbox; the recorder
        // starts focused so Space re-records without touching the mouse.
        window.setAutorecalculatesKeyViewLoop(true);
        if let Some(recorder) = ivars.recorder.borrow().as_ref() {
            window.setInitialFirstResponder(Some(recorder));
        }
        self.fit_settings();
        window.center();
        window.makeKeyAndOrderFront(None);
        NSApplication::sharedApplication(self.mtm()).activate();
    }

    fn refresh_login_checkbox(&self) {
        let ivars = self.ivars();
        let enabled = crate::login::is_enabled();
        if let Some(checkbox) = ivars.login_checkbox.borrow().as_ref() {
            checkbox.setState(if enabled {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
        }
    }

    fn settings_status(&self, message: &str) {
        self.settings_message(message, &NSColor::secondaryLabelColor());
    }

    /// A failure in Settings: red text whose words stand on their own.
    fn settings_error(&self, message: &str) {
        self.settings_message(message, &NSColor::systemRedColor());
    }

    fn settings_message(&self, message: &str, color: &NSColor) {
        if let Some(label) = self.ivars().settings_status.borrow().as_ref() {
            label.setTextColor(Some(color));
            label.setStringValue(&ns_string_from(message));
            label.setHidden(message.is_empty());
        }
        self.fit_settings();
    }

    /// Resizes the Settings window to its content's fitting size.
    fn fit_settings(&self) {
        let Some(window) = self.ivars().settings.borrow().clone() else {
            return;
        };
        let Some(content) = window.contentView() else {
            return;
        };
        content.layoutSubtreeIfNeeded();
        let size = content.fittingSize();
        window.setContentSize(size);
    }

    fn show(&self) {
        debuglog::event("show");
        self.show_search();
        let failed = matches!(*self.ivars().dictionary.borrow(), Dictionary::Failed(_));
        if failed {
            // Reopening the window is the retry: no extra control to find.
            self.spawn_dictionary_job();
        }
        self.show_dictionary_status();
        self.fit_popover();
        let ivars = self.ivars();
        let bounds = ivars.anchor.bounds();
        ivars.popover.showRelativeToRect_ofView_preferredEdge(
            bounds,
            &ivars.anchor,
            NSRectEdge::MaxY,
        );
        self.focus_field();
        NSApplication::sharedApplication(self.mtm()).activate();
    }

    /// Opens the dictionary if it is on disk (so search works at once) and
    /// starts the background fetch: the first download when missing, the
    /// monthly refresh otherwise. Never blocks the main thread on network.
    pub(crate) fn start_dictionary(&self) {
        let path = vocab_capture::dictionary_path(None);
        if path.is_file() {
            *self.ivars().dictionary.borrow_mut() = match open_dictionary(&path) {
                Ok(service) => Dictionary::Ready(service),
                Err(err) => Dictionary::Failed(err),
            };
        }
        self.spawn_dictionary_job();
    }

    fn spawn_dictionary_job(&self) {
        let ivars = self.ivars();
        if ivars.dictionary_job.borrow().is_some() {
            return;
        }
        let path = vocab_capture::dictionary_path(None);
        let before = modified_at(&path);
        let (sender, done) = mpsc::channel();
        let worker_path = path.clone();
        let spawned = std::thread::Builder::new()
            .name(String::from("dictionary-fetch"))
            .spawn(move || {
                let result = vocab_dictionary::ensure_dictionary_db(&worker_path)
                    .map_err(|err| err.to_string());
                // The receiver only goes away with the app; nothing to do then.
                let _ = sender.send(result);
            });
        if let Err(err) = spawned {
            debuglog::event(&format!("dictionary thread error {err}"));
            let mut dictionary = ivars.dictionary.borrow_mut();
            if !matches!(*dictionary, Dictionary::Ready(_)) {
                *dictionary = Dictionary::Failed(format!("the download couldn't start ({err})"));
            }
            return;
        }
        {
            let mut dictionary = ivars.dictionary.borrow_mut();
            if matches!(*dictionary, Dictionary::Failed(_)) && !path.is_file() {
                *dictionary = Dictionary::Loading;
            }
        }
        debuglog::event("dictionary job started");
        // SAFETY: repeating target/selector timer; the target lives for the
        // process lifetime, and the timer is invalidated when the job ends.
        let poll = unsafe {
            NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                DICTIONARY_POLL_INTERVAL,
                self,
                sel!(pollDictionary:),
                None,
                true,
            )
        };
        *ivars.dictionary_job.borrow_mut() = Some(DictionaryJob { done, poll, before });
    }

    fn finish_dictionary_job_if_done(&self) {
        let ivars = self.ivars();
        let received = match ivars.dictionary_job.borrow().as_ref() {
            Some(job) => job.done.try_recv(),
            None => return,
        };
        let outcome = match received {
            Ok(outcome) => outcome,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err(String::from("the download stopped unexpectedly"))
            }
        };
        let Some(job) = ivars.dictionary_job.borrow_mut().take() else {
            return;
        };
        job.poll.invalidate();
        let path = vocab_capture::dictionary_path(None);
        let replaced = modified_at(&path) != job.before;
        let ready = matches!(*ivars.dictionary.borrow(), Dictionary::Ready(_));
        match outcome {
            // A working dictionary survives a failed refresh or reopen.
            Ok(()) if !ready || replaced => match open_dictionary(&path) {
                Ok(service) => *ivars.dictionary.borrow_mut() = Dictionary::Ready(service),
                Err(err) if !ready => *ivars.dictionary.borrow_mut() = Dictionary::Failed(err),
                Err(err) => debuglog::event(&format!("dictionary reopen error {err}")),
            },
            Ok(()) => {}
            Err(err) if !ready => *ivars.dictionary.borrow_mut() = Dictionary::Failed(err),
            Err(err) => debuglog::event(&format!("dictionary refresh error {err}")),
        }
        debuglog::event(&format!("dictionary job finished (replaced={replaced})"));
        self.show_dictionary_status();
        if matches!(*ivars.dictionary.borrow(), Dictionary::Ready(_)) {
            if ivars.popover.isShown() {
                // Anything typed while waiting gets its results now.
                self.run_search_now();
            } else {
                // Don't greet the next open with a stale waiting message.
                self.set_status(ns_string!(""));
            }
        }
    }

    /// Placeholder and status line for the dictionary state. Only speaks up
    /// when search cannot run; a ready dictionary leaves both as they were.
    fn show_dictionary_status(&self) {
        let ivars = self.ivars();
        let message = match &*ivars.dictionary.borrow() {
            Dictionary::Ready(_) => None,
            Dictionary::Loading => Some((
                false,
                String::from(
                    "Downloading the dictionary. This happens once and can take a minute; \
                     results appear as soon as it's ready.",
                ),
            )),
            Dictionary::Failed(err) => Some((
                true,
                format!(
                    "Couldn't load the dictionary: {err}. Check your internet connection, \
                     then reopen this window to try again.",
                ),
            )),
        };
        if let Some((failed, message)) = message {
            ivars
                .field
                .setPlaceholderString(Some(ns_string!("Waiting for the dictionary…")));
            if failed {
                self.set_error(&message);
            } else {
                self.set_status(&ns_string_from(&message));
            }
            ivars.status.setToolTip(Some(&ns_string_from(&message)));
        } else {
            ivars
                .field
                .setPlaceholderString(Some(ns_string!("pinyin, English, or 中文…")));
            ivars.status.setToolTip(None);
        }
    }

    /// Asks `VoiceOver` to speak `text` without moving focus.
    fn announce(&self, text: &str) {
        let message = ns_string_from(text);
        let value: &AnyObject = &message;
        // SAFETY: extern statics provided by AppKit; read-only.
        let (key, notification) = unsafe {
            (
                NSAccessibilityAnnouncementKey,
                NSAccessibilityAnnouncementRequestedNotification,
            )
        };
        let info = NSDictionary::from_slices(&[key], &[value]);
        let element: &AnyObject = &self.ivars().field;
        // SAFETY: posting a standard notification with a dictionary of the
        // documented key and an `NSString` value.
        unsafe {
            NSAccessibilityPostNotificationWithUserInfo(element, notification, Some(&info));
        }
    }

    /// Deletes the saved item behind a row after confirmation. Deletion is
    /// irreversible (Archive is the reversible alternative), so Cancel is the
    /// default button and Delete is marked destructive.
    fn delete_row(&self, index: usize) {
        let ivars = self.ivars();
        let Some(hit) = ivars.results.borrow().get(index).cloned() else {
            return;
        };
        let item_id = match find_saved_reading(
            &ivars.user_conn,
            &hit.entry.simplified,
            &hit.entry.traditional,
            &hit.entry.pinyin,
        ) {
            Ok(Some(item)) => item.item_id,
            Ok(None) => {
                self.set_status(&ns_string_from(&format!(
                    "{} isn't in your list yet. Press Return to save it first.",
                    hit.entry.simplified
                )));
                return;
            }
            Err(err) => {
                self.set_error(&format!("Couldn't look up {}: {err}", hit.entry.simplified));
                return;
            }
        };
        let alert = NSAlert::new(self.mtm());
        alert.setMessageText(&ns_string_from(&format!(
            "Delete {} from your list?",
            hit.entry.simplified
        )));
        alert.setInformativeText(ns_string!(
            "You can't undo this. To keep it out of the way without losing it, choose Archive instead."
        ));
        // First button is the default: Return keeps the word.
        alert.addButtonWithTitle(ns_string!("Cancel"));
        let delete = alert.addButtonWithTitle(ns_string!("Delete"));
        delete.setHasDestructiveAction(true);
        if alert.runModal() != NSAlertSecondButtonReturn {
            return;
        }
        match delete_item(&ivars.user_conn, item_id) {
            Ok(()) => {
                debuglog::event(&format!(
                    "deleted item {item_id} ({})",
                    hit.entry.simplified
                ));
                self.rebuild_rows();
                self.select_row(index);
                self.set_status(&ns_string_from(&format!(
                    "Deleted {} from your list.",
                    hit.entry.simplified
                )));
            }
            Err(err) => {
                debuglog::event(&format!("delete error {err}"));
                self.set_error(&format!("Couldn't delete {}: {err}", hit.entry.simplified));
            }
        }
    }

    fn hide(&self) {
        debuglog::event("hide reason=esc-or-action");
        self.ivars().popover.close();
    }

    fn focus_field(&self) {
        let ivars = self.ivars();
        if let Some(window) = ivars.field.window() {
            let ok = window.makeFirstResponder(Some(&ivars.field));
            debuglog::event(&format!("focus field -> {ok}"));
        } else {
            debuglog::event("focus field -> no window");
        }
    }

    fn show_search(&self) {
        let ivars = self.ivars();
        if let Some(timer) = ivars.dismiss_timer.borrow_mut().take() {
            timer.invalidate();
        }
        ivars.success.setHidden(true);
        ivars.scroll.setHidden(ivars.rows.borrow().is_empty());
        ivars
            .detail
            .setHidden(ivars.detail.stringValue().length() == 0);
        self.fit_popover();
    }

    /// Sizes the popover to its content: the root stack's fitting height at
    /// the fixed width. Resizes without animation so typing never waits on
    /// a transition.
    fn fit_popover(&self) {
        let ivars = self.ivars();
        ivars.content.layoutSubtreeIfNeeded();
        let fitting = ivars.content.fittingSize();
        let target = NSSize::new(CONTENT_WIDTH, fitting.height);
        let current = ivars.popover.contentSize();
        if (current.height - target.height).abs() > 0.5
            || (current.width - target.width).abs() > 0.5
        {
            ivars.popover.setAnimates(false);
            ivars.popover.setContentSize(target);
            ivars.popover.setAnimates(true);
        }
    }

    /// Status line text. Empty text hides the line so the popover shrinks.
    fn set_status(&self, text: &NSString) {
        let status = &self.ivars().status;
        status.setTextColor(Some(&NSColor::secondaryLabelColor()));
        status.setStringValue(text);
        status.setHidden(text.length() == 0);
        self.fit_popover();
    }

    /// A failure on the status line: what failed, then the reason. Red, but
    /// the words ("Couldn't …") carry the meaning on their own.
    fn set_error(&self, text: &str) {
        self.set_status(&ns_string_from(text));
        self.ivars()
            .status
            .setTextColor(Some(&NSColor::systemRedColor()));
        self.announce(text);
    }

    /// Detail text (the selected row's full definition). Empty hides it.
    fn set_detail(&self, text: &NSString) {
        let ivars = self.ivars();
        ivars.detail.setStringValue(text);
        ivars
            .detail
            .setHidden(text.length() == 0 || !ivars.success.isHidden());
        self.fit_popover();
    }

    fn schedule_search(&self) {
        let ivars = self.ivars();
        if let Some(timer) = ivars.search_timer.borrow_mut().take() {
            timer.invalidate();
        }
        // SAFETY: target/selector timer; the target is `self`, which lives
        // for the process lifetime in the shared slot, and the timer is
        // invalidated (or fires once) before replacement.
        let timer = unsafe {
            NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                DEBOUNCE_INTERVAL,
                self,
                sel!(runDebouncedSearch:),
                None,
                false,
            )
        };
        *ivars.search_timer.borrow_mut() = Some(timer);
    }

    fn run_search_now(&self) {
        let ivars = self.ivars();
        let query = ivars.field.stringValue().to_string();
        let query = query.trim().to_owned();
        if query.is_empty() {
            *ivars.results.borrow_mut() = Vec::new();
            *ivars.selection.borrow_mut() = 0;
            self.rebuild_rows();
            self.set_detail(ns_string!(""));
            self.set_status(ns_string!(""));
            return;
        }
        let searched = match &*ivars.dictionary.borrow() {
            Dictionary::Ready(service) => Some(search_auto(service, &query, RESULT_LIMIT)),
            Dictionary::Loading | Dictionary::Failed(_) => None,
        };
        let Some(searched) = searched else {
            // No dictionary yet: keep the query, explain why nothing shows.
            self.show_dictionary_status();
            return;
        };
        match searched {
            Ok((hits, mode)) => {
                debuglog::event(&format!(
                    "search {query:?} -> {} hit(s) via {mode}",
                    hits.len()
                ));
                *ivars.results.borrow_mut() = hits;
                *ivars.selection.borrow_mut() = 0;
                self.rebuild_rows();
                if ivars.results.borrow().is_empty() {
                    self.set_detail(ns_string!(""));
                    let message = format!(
                        "No matches for “{query}”. Try another spelling, English, or characters."
                    );
                    self.set_status(&ns_string_from(&message));
                    self.announce(&message);
                } else {
                    self.select_row(0);
                    let count = ivars.results.borrow().len();
                    self.set_status(&ns_string_from(&results_summary(count, mode)));
                }
            }
            Err(err) => {
                debuglog::event(&format!("search {query:?} -> error"));
                *ivars.results.borrow_mut() = Vec::new();
                self.rebuild_rows();
                self.set_detail(ns_string!(""));
                self.set_error(&format!("Couldn't search the dictionary: {err}"));
            }
        }
    }

    /// Rebuilds result rows from the current results. Old rows are removed
    /// from the view hierarchy (which releases `AppKit`'s retains) and
    /// dropped here; no row outlives this refresh.
    fn rebuild_rows(&self) {
        let ivars = self.ivars();
        {
            let mut rows = ivars.rows.borrow_mut();
            for row in rows.iter() {
                row.view.removeFromSuperview();
            }
            rows.clear();
        }
        let results = ivars.results.borrow();
        let mtm = self.mtm();
        let mut rows = ivars.rows.borrow_mut();
        for (index, hit) in results.iter().enumerate() {
            let BuiltRow {
                view,
                highlight,
                headline,
                reading,
                tag,
                gloss,
                click,
            } = build_row(
                mtm,
                &crate::capture::headword(hit),
                &crate::capture::reading(hit),
                &crate::capture::gloss_line(hit),
            );
            click.setTag(NSInteger::try_from(index).unwrap_or(0));
            // SAFETY: target/action; the delegate outlives every row.
            unsafe {
                click.setTarget(Some(self));
                click.setAction(Some(sel!(selectRow:)));
            }
            let saved = find_saved_item(
                &ivars.user_conn,
                &hit.entry.simplified,
                &hit.entry.traditional,
            )
            .ok()
            .flatten()
            .map(|item| item.status);
            // The click target is the row's one accessibility element; the
            // labels under it are presentation only. The same text is the
            // tooltip, so truncated rows can still be read in full.
            let description = ns_string_from(&row_description(hit, saved.is_some()));
            for label in [&headline, &reading, &tag, &gloss] {
                label.setAccessibilityElement(false);
            }
            click.setAccessibilityLabel(Some(&description));
            click.setToolTip(Some(&description));
            // SAFETY: `-[NSView setMenu:]` is a plain setter absent from the
            // generated bindings; both objects are live and the menu is
            // retained by the view.
            let menu = Self::row_menu(mtm, self, index);
            unsafe {
                let _: () = msg_send![&view, setMenu: Some(&*menu)];
            }
            add_full_width(&ivars.list, &view, 0.0);
            rows.push(RowViews {
                view,
                click,
                highlight,
                headline,
                reading,
                gloss,
                tag,
                saved,
            });
        }
        // The list is as tall as its rows, up to VISIBLE_ROWS, then scrolls.
        ivars.list.layoutSubtreeIfNeeded();
        let content_height = ivars.list.fittingSize().height;
        let row_height = rows
            .first()
            .map_or(0.0, |row| row.view.fittingSize().height);
        ivars
            .list_height
            .setConstant(content_height.min(row_height * VISIBLE_ROWS));
        let empty = rows.is_empty();
        drop(rows);
        drop(results);
        ivars.scroll.setHidden(empty || !ivars.success.isHidden());
        self.fit_popover();
    }

    /// Right-click menu for one result row. The item tags carry the row
    /// index; status operations resolve it to an item id at click time, so
    /// a stale menu can never retarget a different row.
    fn row_menu(mtm: MainThreadMarker, delegate: &Self, index: usize) -> Retained<NSMenu> {
        let menu = NSMenu::new(mtm);
        let tag = NSInteger::try_from(index).unwrap_or(0);
        // Shortcuts mirror the main menu's Word items (see `menus`), shown
        // here so right-click teaches the keyboard path.
        let command = NSEventModifierFlags::Command;
        for (title, action, key, modifiers) in [
            (
                "Save",
                sel!(saveRowFromMenu:),
                "\r",
                NSEventModifierFlags::empty(),
            ),
            (
                "Mark as Needs Review",
                sel!(markReviewFromMenu:),
                "r",
                command,
            ),
            (
                "Archive",
                sel!(archiveFromMenu:),
                "a",
                command | NSEventModifierFlags::Control,
            ),
        ] {
            // SAFETY: standard initializer; the item is retained by the menu.
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    mtm.alloc(),
                    &ns_string_from(title),
                    Some(action),
                    &ns_string_from(key),
                )
            };
            item.setKeyEquivalentModifierMask(modifiers);
            item.setTag(tag);
            // SAFETY: the delegate outlives every menu (process lifetime).
            unsafe {
                item.setTarget(Some(delegate));
            }
            menu.addItem(&item);
        }
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        // SAFETY: standard initializer (see above).
        let delete = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                mtm.alloc(),
                ns_string!("Delete…"),
                Some(sel!(deleteFromMenu:)),
                &ns_string_from(crate::menus::DELETE_KEY),
            )
        };
        delete.setKeyEquivalentModifierMask(
            NSEventModifierFlags::Command | NSEventModifierFlags::Option,
        );
        delete.setTag(tag);
        unsafe {
            delete.setTarget(Some(delegate));
        }
        menu.addItem(&delete);
        menu
    }

    fn move_selection(&self, delta: isize) {
        let len = self.ivars().results.borrow().len();
        let Ok(len) = isize::try_from(len) else {
            return;
        };
        if len <= 0 {
            return;
        }
        let current = isize::try_from(*self.ivars().selection.borrow()).unwrap_or(0);
        let next = (current + delta).rem_euclid(len);
        self.select_row(usize::try_from(next).unwrap_or(0));
    }

    fn select_row(&self, index: usize) {
        let ivars = self.ivars();
        if ivars.results.borrow().get(index).is_none() {
            return;
        }
        *ivars.selection.borrow_mut() = index;
        let rows = ivars.rows.borrow();
        for (position, row) in rows.iter().enumerate() {
            let selected = position == index;
            // Saved entries say so on every row; the ↩ Save hint appears
            // only on the selected unsaved row. The status itself lives in
            // the detail area and review flows, not on the row.
            if row.saved.is_some() {
                row.tag.setStringValue(ns_string!("Saved"));
                row.tag.setHidden(false);
            } else {
                row.tag.setStringValue(ns_string!("↩ Save"));
                row.tag.setHidden(!selected);
            }
            // Native list selection: the accent fill with inverted text,
            // instead of a weight change that shifts truncation.
            row.highlight.setHidden(!selected);
            let (primary, secondary) = if selected {
                let on_accent = NSColor::alternateSelectedControlTextColor();
                (on_accent.clone(), on_accent)
            } else {
                (NSColor::labelColor(), NSColor::secondaryLabelColor())
            };
            row.headline.setTextColor(Some(&primary));
            row.tag.setTextColor(Some(&primary));
            row.reading.setTextColor(Some(&secondary));
            row.gloss.setTextColor(Some(&secondary));
            row.click.setAccessibilitySelected(selected);
        }
        if let Some(selected) = rows.get(index) {
            selected.view.scrollRectToVisible(selected.view.bounds());
        }
        drop(rows);
        let saved = rows_saved(&ivars.rows.borrow(), index);
        let spoken = ivars.results.borrow().get(index).map(|hit| {
            self.set_detail(&ns_string_from(&crate::capture::gloss_line(hit)));
            row_description(hit, saved)
        });
        // Focus stays in the field while arrows move the selection, so
        // VoiceOver hears the new row only through an announcement.
        if let Some(spoken) = spoken {
            self.announce(&spoken);
        }
    }

    fn save_selected(&self) {
        let index = *self.ivars().selection.borrow();
        self.save_row(index);
    }

    fn save_row(&self, index: usize) {
        let ivars = self.ivars();
        let Some(hit) = ivars.results.borrow().get(index).cloned() else {
            return;
        };
        debuglog::event(&format!("save start simplified={}", hit.entry.simplified));
        self.set_status(&ns_string_from(&format!(
            "Saving {}…",
            hit.entry.simplified
        )));
        match save_candidate(&ivars.user_conn, &hit) {
            Ok(SaveOutcome::Inserted(item)) => {
                self.set_status(ns_string!(""));
                self.show_success(
                    "Saved",
                    &format!(
                        "{} ({}) is in your list.",
                        item.simplified,
                        vocab_pinyin::tone_marks(&item.pinyin)
                    ),
                );
            }
            Ok(SaveOutcome::Duplicate(item)) => {
                self.set_status(ns_string!(""));
                self.show_success(
                    "Already in your list",
                    &format!(
                        "You saved {} ({}) earlier; it's marked {}.",
                        item.simplified,
                        vocab_pinyin::tone_marks(&item.pinyin),
                        item.status.label()
                    ),
                );
            }
            Err(err) => {
                debuglog::event(&format!("save error {err}"));
                self.set_error(&format!("Couldn't save {}: {err}", hit.entry.simplified));
            }
        }
    }

    /// Applies `status` to the saved item behind a menu row. Re-reads the
    /// item id at click time (never trusts the badge) and refreshes badges
    /// afterwards; the dictionary results themselves are unchanged.
    fn set_status_for_row(&self, index: usize, status: ItemStatus) {
        let ivars = self.ivars();
        let Some(hit) = ivars.results.borrow().get(index).cloned() else {
            return;
        };
        let item_id = match find_saved_reading(
            &ivars.user_conn,
            &hit.entry.simplified,
            &hit.entry.traditional,
            &hit.entry.pinyin,
        ) {
            Ok(Some(item)) => item.item_id,
            Ok(None) => {
                self.set_status(&ns_string_from(&format!(
                    "{} isn't in your list yet. Press Return to save it first.",
                    hit.entry.simplified
                )));
                return;
            }
            Err(err) => {
                self.set_error(&format!("Couldn't look up {}: {err}", hit.entry.simplified));
                return;
            }
        };
        match set_status(&ivars.user_conn, item_id, status) {
            Ok(()) => {
                debuglog::event(&format!(
                    "status {} item {item_id} ({})",
                    status.as_str(),
                    hit.entry.simplified
                ));
                self.rebuild_rows();
                self.select_row(index);
                self.set_status(&ns_string_from(&match status {
                    ItemStatus::Archived => format!("Archived {}.", hit.entry.simplified),
                    other => format!("Marked {} as {}.", hit.entry.simplified, other.label()),
                }));
            }
            Err(err) => {
                debuglog::event(&format!("status error {err}"));
                self.set_error(&format!(
                    "Couldn't mark {} as {}: {err}",
                    hit.entry.simplified,
                    status.label()
                ));
            }
        }
    }

    fn show_success(&self, title: &str, message: &str) {
        debuglog::event("save confirmation shown");
        let ivars = self.ivars();
        ivars.success_title.setStringValue(&ns_string_from(title));
        ivars
            .success_message
            .setStringValue(&ns_string_from(message));
        ivars.scroll.setHidden(true);
        ivars.detail.setHidden(true);
        ivars.success.setHidden(false);
        // Size first: the animations below start from the laid-out position.
        self.fit_popover();
        // Fade the container: closed (alpha 0) → open (alpha 1).
        // No completion block: sequencing continues on an `NSTimer`.
        NSAnimationContext::beginGrouping();
        NSAnimationContext::currentContext().setDuration(FADE_DURATION);
        ivars.success.animator().setAlphaValue(1.0);
        NSAnimationContext::endGrouping();
        // With Reduce Motion on, the fade alone confirms the save: no scale,
        // slide, or spring.
        let reduce_motion = NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion();
        // Scale .96 → 1 with momentum past the end, per the closed→open
        // recipe. Layer-backed views were enabled at construction.
        if let Some(layer) = ivars.success.layer().filter(|_| !reduce_motion) {
            let scale = CABasicAnimation::animationWithKeyPath(Some(ns_string!("transform.scale")));
            unsafe {
                scale.setFromValue(Some(&NSNumber::numberWithDouble(0.96)));
                scale.setToValue(Some(&NSNumber::numberWithDouble(1.0)));
            }
            scale.setDuration(FADE_DURATION);
            layer.addAnimation_forKey(&scale, None);
            let position = layer.position();
            let slide = CABasicAnimation::animationWithKeyPath(Some(ns_string!("position.y")));
            unsafe {
                slide.setFromValue(Some(&NSNumber::numberWithDouble(position.y - 4.0)));
                slide.setToValue(Some(&NSNumber::numberWithDouble(position.y)));
            }
            slide.setDuration(FADE_DURATION);
            layer.addAnimation_forKey(&slide, None);
        }
        // The check pops with a spring while the container fades in.
        if let Some(layer) = ivars.check.layer().filter(|_| !reduce_motion) {
            let pop = CASpringAnimation::animationWithKeyPath(Some(ns_string!("transform.scale")));
            pop.setMass(1.0);
            pop.setStiffness(220.0);
            pop.setDamping(14.0);
            unsafe {
                pop.setFromValue(Some(&NSNumber::numberWithDouble(0.5)));
                pop.setToValue(Some(&NSNumber::numberWithDouble(1.0)));
            }
            layer.addAnimation_forKey(&pop, None);
        }
        if let Some(timer) = ivars.dismiss_timer.borrow_mut().take() {
            timer.invalidate();
        }
        // SAFETY: same contract as the search timer above.
        let timer = unsafe {
            NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                DISMISS_HOLD,
                self,
                sel!(dismissAfterHold:),
                None,
                false,
            )
        };
        *ivars.dismiss_timer.borrow_mut() = Some(timer);
    }
}

thread_local! {
    static DELEGATE: RefCell<Option<Retained<CaptureDelegate>>> =
        const { RefCell::new(None) };
}

/// Installs the shared controller. Call once, on the main thread, before
/// `NSApplication::run`. Returns it for menu wiring.
pub fn install(
    mtm: MainThreadMarker,
    anchor: Retained<NSStatusBarButton>,
    user_conn: rusqlite::Connection,
) -> Retained<CaptureDelegate> {
    let delegate = CaptureDelegate::new(mtm, anchor, user_conn);
    DELEGATE.with(|slot| {
        *slot.borrow_mut() = Some(delegate.clone());
    });
    delegate
}

/// Toggles the popover from the hotkey callback (main thread by contract).
pub fn toggle_shared() {
    DELEGATE.with(|slot| {
        if let Some(delegate) = slot.borrow().as_ref() {
            delegate.toggle();
        }
    });
}

/// Applies a freshly recorded hotkey: validates, persists, re-registers,
/// and refreshes the recorder label. Called from the recorder view.
pub(crate) fn apply_recorded_hotkey(binding: crate::settings::HotkeyBinding, display: &str) {
    if !crate::settings::valid_hotkey(binding.modifiers) {
        return;
    }
    crate::settings::store_binding(binding, display);
    DELEGATE.with(|slot| {
        if let Some(delegate) = slot.borrow().as_ref() {
            delegate.rebind_hotkey(binding);
            if let Some(recorder) = delegate.ivars().recorder.borrow().as_ref() {
                recorder.refresh_display();
            }
            debuglog::event(&format!(
                "hotkey rebound to {display} (code {}, mods {:#x})",
                binding.keycode, binding.modifiers
            ));
        }
    });
}
