//! Native macOS menu-bar utility (`AppKit` via `objc2`).
//!
//! Workspace mapping for the suggested `core / tui / macos` split: the
//! `vocab-*` workspace crates are the platform-independent core, `shouci-tui`
//! is the terminal UI, and this crate is the `macos/` layer. It links the
//! same core code the CLI and TUI use — no subprocesses, no duplicated logic.
//!
//! `unsafe` is allowed in this crate only (workspace override) because
//! Objective-C interop cannot exist without it. All unsafe lives behind
//! small, documented wrappers; business logic stays in safe Rust.

use objc2::rc::Retained;
use objc2::{MainThreadMarker, sel};
use objc2_app_kit::{
    NSAccessibility, NSApplication, NSApplicationActivationPolicy, NSEventMask, NSMenu, NSMenuItem,
    NSStatusBar, NSVariableStatusItemLength,
};
use objc2_foundation::{NSString, ns_string};
use vocab_capture::open_capture_db;
use vocab_db::app_data_dir;

mod capture;
mod debuglog;
mod hotkey;
mod layout;
mod login;
mod menus;
mod settings;
mod ui;

#[cfg(test)]
mod search_quality;

fn main() {
    let mtm = MainThreadMarker::new().expect("shouci-mac must start on the main thread");

    // The user database opens here (local, fast). The dictionary does not:
    // its first download or monthly refresh runs in the background after the
    // status item is up, and the popover reports progress and failures.
    let user_db_path = match app_data_dir() {
        Ok(dir) => dir.join("user.db"),
        Err(_) => std::path::PathBuf::from("user.db"),
    };
    let user_conn = open_capture_db(Some(&user_db_path));
    if let Err(err) = user_conn.as_ref() {
        eprintln!("shouci-mac: user db unavailable: {err}");
        debuglog::event(&format!("user db error {err}"));
    }

    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let status_bar = NSStatusBar::systemStatusBar();
    let item = status_bar.statusItemWithLength(NSVariableStatusItemLength);
    let button = item.button(mtm).expect("status item must have a button");
    button.setTitle(ns_string!("文"));
    // VoiceOver would otherwise read the glyph as "wén".
    button.setAccessibilityLabel(Some(ns_string!("Shouci")));
    button.setToolTip(Some(&NSString::from_str(&format!(
        "Shouci — {} to capture a word",
        settings::display_string()
    ))));

    // The controller owns the popover and all its views. It needs the user
    // database; without it there is nothing to save to.
    let (delegate, user_db_error) = match user_conn {
        Ok(user_conn) => (Some(ui::install(mtm, button.clone(), user_conn)), None),
        Err(err) => {
            eprintln!("shouci-mac: running menu-only: {err}");
            (None, Some(err.to_string()))
        }
    };

    if let Some(delegate) = delegate.as_ref() {
        // Primary click toggles the popover; right-click (or Ctrl-click)
        // shows the delegate-owned menu instead. `item.setMenu` is
        // deliberately never called: with a menu set, AppKit gives it
        // precedence and this action stops firing. Both buttons report to
        // `togglePopover:`, which branches on the event type.
        // SAFETY: same lifetime contract as above.
        unsafe {
            button.setTarget(Some(&**delegate));
            button.setAction(Some(sel!(togglePopover:)));
        }
        button.sendActionOn(NSEventMask::LeftMouseUp | NSEventMask::RightMouseUp);
    } else {
        // Degraded mode (no user database): say why, and offer Quit, so the
        // process is never stranded without UI or an explanation.
        let menu = NSMenu::new(mtm);
        let reason = NSMenuItem::new(mtm);
        reason.setTitle(&NSString::from_str(&format!(
            "Can't open your word list: {}",
            user_db_error.unwrap_or_default()
        )));
        reason.setEnabled(false);
        menu.addItem(&reason);
        let location = NSMenuItem::new(mtm);
        location.setTitle(&NSString::from_str(&user_db_path.display().to_string()));
        location.setEnabled(false);
        menu.addItem(&location);
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        let quit = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                mtm.alloc(),
                ns_string!("Quit Shouci"),
                Some(sel!(terminate:)),
                ns_string!("q"),
            )
        };
        menu.addItem(&quit);
        item.setMenu(Some(&menu));
    }

    // The item must live for the process lifetime. Intentional permanent
    // leak of one small root; the alternative (a global) adds `Sync`
    // questions for a main-thread-only object. (The button itself is owned
    // by the item; the delegate lives in the shared slot in `ui`.)
    let _status_item: *const _ = Retained::into_raw(item);

    // The delegate owns the hotkey registration (it must rebind on settings
    // changes); binding happens here so a failure is visible at startup.
    if let Some(delegate) = delegate.as_ref() {
        delegate.bind_default_hotkey();
        delegate.start_dictionary();
    }
    menus::install(mtm, delegate.as_deref());

    debuglog::event("started");
    app.run();
}
