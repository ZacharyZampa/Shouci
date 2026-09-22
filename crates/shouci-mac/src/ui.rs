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

use objc2::ffi::NSInteger;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAnimatablePropertyContainer, NSAnimationContext,
    NSApplication, NSBackingStoreType, NSButton, NSButtonType, NSColor, NSControl,
    NSControlStateValueOff, NSControlStateValueOn, NSControlTextEditingDelegate, NSEventType,
    NSFont, NSImage, NSImageView, NSMenu, NSMenuItem, NSPopover, NSPopoverBehavior,
    NSPopoverDelegate, NSScrollView, NSStatusBarButton, NSTextField, NSTextFieldDelegate,
    NSTextView, NSView, NSViewController, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{
    NSNotification, NSNumber, NSPoint, NSRect, NSRectEdge, NSSize, NSString, NSTimer, ns_string,
};
use objc2_quartz_core::{CABasicAnimation, CAMediaTiming, CASpringAnimation};
use vocab_capture::{save_candidate, search_auto};
use vocab_core::ItemStatus;
use vocab_db::{SaveOutcome, delete_item, find_saved_item, set_status};
use vocab_dictionary::Candidate;

use crate::capture::RESULT_LIMIT;
use crate::debuglog;

const DEBOUNCE_INTERVAL: f64 = 0.15;
const DISMISS_HOLD: f64 = 1.5;
const FADE_DURATION: f64 = 0.25;
const FADE_OUT_DURATION: f64 = 0.2;
const ROW_HEIGHT: f64 = 44.0;
const LIST_WIDTH: f64 = 376.0;

/// Handles to one result row's live views, plus the saved state behind the
/// badge (`None` = not in the user database).
struct RowViews {
    view: Retained<NSView>,
    headline: Retained<NSTextField>,
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
    scroll: Retained<NSScrollView>,
    list: Retained<NSView>,
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
    service: vocab_search::SearchService<vocab_dictionary::SqliteDictionary>,
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

    // SAFETY: `NSControlTextEditingDelegate` has no safety requirements.
    // The methods below live on the sub-protocol impl; this empty impl
    // satisfies the superclass bound.
    unsafe impl NSControlTextEditingDelegate for CaptureDelegate {}

    // SAFETY: `NSPopoverDelegate` has no safety requirements. Only the
    // close notification is implemented, purely as a diagnostic: it fires
    // for system-driven closes (light-dismiss) that bypass our methods.
    unsafe impl NSPopoverDelegate for CaptureDelegate {
        #[unsafe(method(popoverDidClose:))]
        fn popover_did_close(&self, _notification: &NSNotification) {
            debuglog::event("popover did-close notification");
        }
    }

    // SAFETY: `NSTextFieldDelegate` has no safety requirements; the methods
    // only touch main-thread ivars with scoped borrows.
    unsafe impl NSTextFieldDelegate for CaptureDelegate {
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

        /// Context-menu "Delete…", with a confirmation alert: deletion is
        /// irreversible (archiving is the reversible alternative on the menu).
        #[unsafe(method(deleteFromMenu:))]
        fn delete_from_menu(&self, sender: &NSMenuItem) {
            let Ok(index) = usize::try_from(sender.tag()) else {
                return;
            };
            let ivars = self.ivars();
            let Some(hit) = ivars.results.borrow().get(index).cloned() else {
                return;
            };
            let item_id = match find_saved_item(
                &ivars.user_conn,
                &hit.entry.simplified,
                &hit.entry.traditional,
            ) {
                Ok(Some(item)) => item.item_id,
                Ok(None) => {
                    ivars.status.setStringValue(&ns_string_from(&format!(
                        "not saved yet — press Enter first ({})",
                        hit.entry.simplified
                    )));
                    return;
                }
                Err(err) => {
                    ivars.status.setStringValue(&ns_string_from(&err.to_string()));
                    return;
                }
            };
            let alert = NSAlert::new(self.mtm());
            alert.setMessageText(&ns_string_from(&format!(
                "Delete {}?",
                hit.entry.simplified
            )));
            alert.setInformativeText(&ns_string_from(
                "This removes it from your vocabulary list. This cannot be undone.",
            ));
            alert.addButtonWithTitle(ns_string!("Delete"));
            alert.addButtonWithTitle(ns_string!("Cancel"));
            if alert.runModal() != NSAlertFirstButtonReturn {
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
                    ivars.status.setStringValue(&ns_string_from(&format!(
                        "deleted: {}",
                        hit.entry.simplified
                    )));
                }
                Err(err) => {
                    debuglog::event(&format!("delete error {err}"));
                    ivars.status.setStringValue(&ns_string_from(&err.to_string()));
                }
            }
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
                    self.settings_status(&format!(
                        "Open at Login {}",
                        if enabled { "on" } else { "off" }
                    ));
                }
                Err(message) => {
                    debuglog::event(&format!("login error {message}"));
                    // Revert the checkbox: the change did not take effect.
                    self.refresh_login_checkbox();
                    self.settings_status(&message);
                }
            }
        }
    }
);

/// `NSString` from Rust text (`ns_string!` only takes literals).
fn ns_string_from(text: &str) -> Retained<NSString> {
    NSString::from_str(text)
}

fn build_field(mtm: MainThreadMarker) -> Retained<NSTextField> {
    let field: Retained<NSTextField> = NSTextField::initWithFrame(
        NSTextField::alloc(mtm),
        NSRect::new(NSPoint::new(12.0, 320.0), NSSize::new(376.0, 28.0)),
    );
    field.setEditable(true);
    field.setSelectable(true);
    field.setPlaceholderString(Some(ns_string!("pinyin, English, or 中文…")));
    field.setFont(Some(&NSFont::systemFontOfSize(15.0)));
    field
}

fn build_list(mtm: MainThreadMarker) -> (Retained<NSScrollView>, Retained<NSView>) {
    let scroll: Retained<NSScrollView> = NSScrollView::initWithFrame(
        NSScrollView::alloc(mtm),
        NSRect::new(NSPoint::new(12.0, 92.0), NSSize::new(376.0, 216.0)),
    );
    scroll.setHasVerticalScroller(true);
    let list: Retained<NSView> = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(LIST_WIDTH, 216.0)),
    );
    scroll.setDocumentView(Some(&list));
    (scroll, list)
}

fn build_labels(mtm: MainThreadMarker) -> (Retained<NSTextField>, Retained<NSTextField>) {
    let detail = NSTextField::labelWithString(ns_string!(""), mtm);
    detail.setFrame(NSRect::new(
        NSPoint::new(12.0, 52.0),
        NSSize::new(376.0, 34.0),
    ));
    detail.setFont(Some(&NSFont::systemFontOfSize(12.0)));
    detail.setTextColor(Some(&NSColor::secondaryLabelColor()));

    let status = NSTextField::labelWithString(ns_string!(""), mtm);
    status.setFrame(NSRect::new(
        NSPoint::new(12.0, 12.0),
        NSSize::new(376.0, 28.0),
    ));
    status.setFont(Some(&NSFont::systemFontOfSize(11.0)));
    status.setTextColor(Some(&NSColor::tertiaryLabelColor()));
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
    let success: Retained<NSView> = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(12.0, 52.0), NSSize::new(376.0, 256.0)),
    );
    success.setWantsLayer(true);
    success.setAlphaValue(0.0);
    success.setHidden(true);
    let check: Retained<NSImageView> = NSImageView::initWithFrame(
        NSImageView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 200.0), NSSize::new(44.0, 44.0)),
    );
    if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        ns_string!("checkmark.circle.fill"),
        Some(ns_string!("saved")),
    ) {
        check.setImage(Some(&image));
    }
    check.setWantsLayer(true);
    let success_title = NSTextField::labelWithString(ns_string!("✓ Saved"), mtm);
    success_title.setFrame(NSRect::new(
        NSPoint::new(56.0, 218.0),
        NSSize::new(320.0, 24.0),
    ));
    success_title.setFont(Some(&NSFont::boldSystemFontOfSize(15.0)));
    let success_message = NSTextField::labelWithString(ns_string!(""), mtm);
    success_message.setFrame(NSRect::new(
        NSPoint::new(56.0, 190.0),
        NSSize::new(320.0, 24.0),
    ));
    success_message.setFont(Some(&NSFont::systemFontOfSize(12.0)));
    success_message.setTextColor(Some(&NSColor::secondaryLabelColor()));
    success.addSubview(&check);
    success.addSubview(&success_title);
    success.addSubview(&success_message);
    (success, check, success_title, success_message)
}

impl CaptureDelegate {
    fn new(
        mtm: MainThreadMarker,
        anchor: Retained<NSStatusBarButton>,
        service: vocab_search::SearchService<vocab_dictionary::SqliteDictionary>,
        user_conn: rusqlite::Connection,
    ) -> Retained<Self> {
        let popover = NSPopover::new(mtm);
        popover.setBehavior(NSPopoverBehavior::Transient);

        // SAFETY: allocating views then running the designated initializer
        // before any other use; all objects stay owned by ivars/views.
        unsafe {
            let content: Retained<NSView> = NSView::initWithFrame(
                NSView::alloc(mtm),
                NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(400.0, 360.0)),
            );
            let field = build_field(mtm);
            let (scroll, list) = build_list(mtm);
            let (detail, status) = build_labels(mtm);
            let (success, check, success_title, success_message) = build_success(mtm);

            content.addSubview(&field);
            content.addSubview(&scroll);
            content.addSubview(&detail);
            content.addSubview(&status);
            content.addSubview(&success);

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
                ns_string!(""),
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
                scroll,
                list,
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
                service,
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
                self.settings_status("Hotkey updated — try it anywhere.");
            }
            Err(err) => {
                debuglog::event(&format!("hotkey error {err}"));
                self.settings_status(&err.to_string());
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

        let hotkey_label = NSTextField::labelWithString(ns_string!("Quick Capture hotkey:"), mtm);
        hotkey_label.setFrame(NSRect::new(
            NSPoint::new(12.0, 206.0),
            NSSize::new(200.0, 20.0),
        ));
        hotkey_label.setFont(Some(&NSFont::systemFontOfSize(13.0)));
        content.addSubview(&hotkey_label);

        let recorder = crate::settings::recorder_view(
            mtm,
            NSRect::new(NSPoint::new(220.0, 202.0), NSSize::new(168.0, 28.0)),
        );
        content.addSubview(&recorder);

        let login = NSButton::initWithFrame(
            NSButton::alloc(mtm),
            NSRect::new(NSPoint::new(12.0, 162.0), NSSize::new(250.0, 24.0)),
        );
        login.setButtonType(NSButtonType::Switch);
        login.setTitle(ns_string!("Open at Login"));
        login.setFont(Some(&NSFont::systemFontOfSize(13.0)));
        // SAFETY: target/action; the delegate outlives the window.
        unsafe {
            login.setTarget(Some(self));
            login.setAction(Some(sel!(toggleLoginItem:)));
        }
        content.addSubview(&login);

        let status = NSTextField::labelWithString(ns_string!(""), mtm);
        status.setFrame(NSRect::new(
            NSPoint::new(12.0, 12.0),
            NSSize::new(376.0, 130.0),
        ));
        status.setFont(Some(&NSFont::systemFontOfSize(11.0)));
        status.setTextColor(Some(&NSColor::secondaryLabelColor()));
        content.addSubview(&status);

        *ivars.recorder.borrow_mut() = Some(recorder);
        *ivars.settings_status.borrow_mut() = Some(status);
        *ivars.login_checkbox.borrow_mut() = Some(login);
        *ivars.settings.borrow_mut() = Some(window.clone());
        // `ivars` is a shared borrow; it ends here by NLL before the calls
        // below re-borrow through `self`.
        self.refresh_login_checkbox();
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
        if let Some(label) = self.ivars().settings_status.borrow().as_ref() {
            label.setStringValue(&ns_string_from(message));
        }
    }

    fn show(&self) {
        debuglog::event("show");
        self.show_search();
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
        ivars.scroll.setHidden(false);
        ivars.detail.setHidden(false);
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
            ivars.detail.setStringValue(ns_string!(""));
            ivars.status.setStringValue(ns_string!(""));
            return;
        }
        match search_auto(&ivars.service, &query, RESULT_LIMIT) {
            Ok((hits, mode)) => {
                debuglog::event(&format!(
                    "search {query:?} -> {} hit(s) via {mode}",
                    hits.len()
                ));
                *ivars.results.borrow_mut() = hits;
                *ivars.selection.borrow_mut() = 0;
                self.rebuild_rows();
                if ivars.results.borrow().is_empty() {
                    ivars.detail.setStringValue(ns_string!(""));
                    ivars
                        .status
                        .setStringValue(ns_string!("No results — try another spelling."));
                } else {
                    self.select_row(0);
                    let count = ivars.results.borrow().len();
                    ivars
                        .status
                        .setStringValue(&ns_string_from(&format!("{count} result(s) · {mode}")));
                }
            }
            Err(err) => {
                debuglog::event(&format!("search {query:?} -> error"));
                *ivars.results.borrow_mut() = Vec::new();
                self.rebuild_rows();
                ivars.detail.setStringValue(ns_string!(""));
                ivars
                    .status
                    .setStringValue(&ns_string_from(&err.to_string()));
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
        let count = results.len();
        let height = (f64::from(u32::try_from(count).unwrap_or(u32::MAX)) * ROW_HEIGHT).max(216.0);
        ivars.list.setFrameSize(NSSize::new(LIST_WIDTH, height));
        let mtm = self.mtm();
        let mut rows = ivars.rows.borrow_mut();
        for (index, hit) in results.iter().enumerate() {
            // `AppKit` origin is bottom-left: anchor the first hit at the
            // top of the (possibly taller than the scroll view) container.
            let from_top = u32::try_from(index).unwrap_or(u32::MAX);
            let y = height - (f64::from(from_top) + 1.0) * ROW_HEIGHT;
            // SAFETY: same allocate-then-init contract as construction.
            let view: Retained<NSView> = NSView::initWithFrame(
                NSView::alloc(mtm),
                NSRect::new(NSPoint::new(0.0, y), NSSize::new(LIST_WIDTH, ROW_HEIGHT)),
            );
            let headline: Retained<NSTextField> = NSTextField::initWithFrame(
                NSTextField::alloc(mtm),
                NSRect::new(NSPoint::new(8.0, 22.0), NSSize::new(272.0, 18.0)),
            );
            headline.setStringValue(&ns_string_from(&crate::capture::headline(hit)));
            headline.setEditable(false);
            headline.setSelectable(false);
            headline.setBordered(false);
            headline.setDrawsBackground(false);
            headline.setFont(Some(&NSFont::systemFontOfSize(13.0)));
            let tag: Retained<NSTextField> = NSTextField::initWithFrame(
                NSTextField::alloc(mtm),
                NSRect::new(NSPoint::new(288.0, 22.0), NSSize::new(80.0, 18.0)),
            );
            tag.setStringValue(ns_string!("↵ Save"));
            tag.setEditable(false);
            tag.setSelectable(false);
            tag.setBordered(false);
            tag.setDrawsBackground(false);
            tag.setFont(Some(&NSFont::boldSystemFontOfSize(11.0)));
            tag.setHidden(true);
            let gloss: Retained<NSTextField> = NSTextField::initWithFrame(
                NSTextField::alloc(mtm),
                NSRect::new(NSPoint::new(8.0, 2.0), NSSize::new(360.0, 16.0)),
            );
            gloss.setStringValue(&ns_string_from(&crate::capture::gloss_line(hit)));
            gloss.setEditable(false);
            gloss.setSelectable(false);
            gloss.setBordered(false);
            gloss.setDrawsBackground(false);
            gloss.setFont(Some(&NSFont::systemFontOfSize(11.0)));
            gloss.setTextColor(Some(&NSColor::secondaryLabelColor()));
            // Invisible click target over the whole row. `tag` carries the
            // result index to the action method; the button itself draws
            // nothing.
            let click: Retained<NSButton> = NSButton::initWithFrame(
                NSButton::alloc(mtm),
                NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(LIST_WIDTH, ROW_HEIGHT)),
            );
            click.setTransparent(true);
            click.setBordered(false);
            click.setTag(NSInteger::try_from(index).unwrap_or(0));
            // SAFETY: target/action; the delegate outlives every row.
            unsafe {
                click.setTarget(Some(self));
                click.setAction(Some(sel!(selectRow:)));
            }
            view.addSubview(&headline);
            view.addSubview(&tag);
            view.addSubview(&gloss);
            view.addSubview(&click);
            // SAFETY: `-[NSView setMenu:]` is a plain setter absent from the
            // generated bindings; both objects are live and the menu is
            // retained by the view.
            let menu = Self::row_menu(mtm, self, index);
            unsafe {
                let _: () = msg_send![&view, setMenu: Some(&*menu)];
            }
            ivars.list.addSubview(&view);
            let saved = find_saved_item(
                &ivars.user_conn,
                &hit.entry.simplified,
                &hit.entry.traditional,
            )
            .ok()
            .flatten()
            .map(|item| item.status);
            rows.push(RowViews {
                view,
                headline,
                tag,
                saved,
            });
        }
    }

    /// Right-click menu for one result row. The item tags carry the row
    /// index; status operations resolve it to an item id at click time, so
    /// a stale menu can never retarget a different row.
    fn row_menu(mtm: MainThreadMarker, delegate: &Self, index: usize) -> Retained<NSMenu> {
        let menu = NSMenu::new(mtm);
        let tag = NSInteger::try_from(index).unwrap_or(0);
        for (title, action) in [
            ("Save", sel!(saveRowFromMenu:)),
            ("Mark as Needs Review", sel!(markReviewFromMenu:)),
            ("Archive", sel!(archiveFromMenu:)),
        ] {
            // SAFETY: standard initializer; the item is retained by the menu.
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    mtm.alloc(),
                    &ns_string_from(title),
                    Some(action),
                    ns_string!(""),
                )
            };
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
                ns_string!(""),
            )
        };
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
            // Saved entries show a plain check; the ↵ Save tag appears only
            // on the selected unsaved row. The status itself lives in the
            // detail area and review flows, not on the row.
            if row.saved.is_some() {
                row.tag.setStringValue(ns_string!("✓"));
                row.tag.setHidden(false);
            } else {
                row.tag.setStringValue(ns_string!("↵ Save"));
                row.tag.setHidden(!selected);
            }
            if selected {
                row.headline
                    .setFont(Some(&NSFont::boldSystemFontOfSize(13.0)));
            } else {
                row.headline.setFont(Some(&NSFont::systemFontOfSize(13.0)));
            }
        }
        if let Some(selected) = rows.get(index) {
            selected.view.scrollRectToVisible(selected.view.bounds());
        }
        drop(rows);
        if let Some(hit) = ivars.results.borrow().get(index) {
            ivars
                .detail
                .setStringValue(&ns_string_from(&crate::capture::gloss_line(hit)));
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
        ivars.status.setStringValue(&ns_string_from(&format!(
            "Saving {}…",
            hit.entry.simplified
        )));
        match save_candidate(&ivars.user_conn, &hit) {
            Ok(SaveOutcome::Inserted(item)) => self.show_success(
                "✓ Saved",
                &format!(
                    "saved: {} / {}  [{}] (item {})",
                    item.simplified, item.traditional, item.pinyin, item.item_id
                ),
            ),
            Ok(SaveOutcome::Duplicate(item)) => self.show_success(
                "Already in your list",
                &format!(
                    "already saved: {} / {}  [{}] (item {})",
                    item.simplified, item.traditional, item.pinyin, item.item_id
                ),
            ),
            Err(err) => {
                debuglog::event(&format!("save error {err}"));
                ivars
                    .status
                    .setStringValue(&ns_string_from(&err.to_string()));
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
        let item_id = match find_saved_item(
            &ivars.user_conn,
            &hit.entry.simplified,
            &hit.entry.traditional,
        ) {
            Ok(Some(item)) => item.item_id,
            Ok(None) => {
                ivars.status.setStringValue(&ns_string_from(&format!(
                    "not saved yet — press Enter first ({})",
                    hit.entry.simplified
                )));
                return;
            }
            Err(err) => {
                ivars
                    .status
                    .setStringValue(&ns_string_from(&err.to_string()));
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
                ivars.status.setStringValue(&ns_string_from(&format!(
                    "marked {} as {}",
                    hit.entry.simplified,
                    status.as_str()
                )));
            }
            Err(err) => {
                debuglog::event(&format!("status error {err}"));
                ivars
                    .status
                    .setStringValue(&ns_string_from(&err.to_string()));
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
        // Fade the container: closed (alpha 0) → open (alpha 1).
        // No completion block: sequencing continues on an `NSTimer`.
        NSAnimationContext::beginGrouping();
        NSAnimationContext::currentContext().setDuration(FADE_DURATION);
        ivars.success.animator().setAlphaValue(1.0);
        NSAnimationContext::endGrouping();
        // Scale .96 → 1 with momentum past the end, per the closed→open
        // recipe. Layer-backed views were enabled at construction.
        if let Some(layer) = ivars.success.layer() {
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
        if let Some(layer) = ivars.check.layer() {
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
    service: vocab_search::SearchService<vocab_dictionary::SqliteDictionary>,
    user_conn: rusqlite::Connection,
) -> Retained<CaptureDelegate> {
    let delegate = CaptureDelegate::new(mtm, anchor, service, user_conn);
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
