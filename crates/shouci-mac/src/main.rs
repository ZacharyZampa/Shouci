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
    NSApplication, NSApplicationActivationPolicy, NSEventMask, NSMenu, NSMenuItem, NSStatusBar,
    NSVariableStatusItemLength,
};
use objc2_foundation::ns_string;
use vocab_capture::{dictionary_path, open_capture_db, open_service};
use vocab_db::app_data_dir;

mod capture;
mod debuglog;
mod hotkey;
mod login;
mod settings;
mod ui;

#[cfg(test)]
mod search_quality;

fn main() {
    let mtm = MainThreadMarker::new().expect("shouci-mac must start on the main thread");

    // Core services first: dictionary search + user database. Failures here
    // do not abort startup; the popover surfaces them in its status line.
    let service = open_service(None);
    let user_db_path = match app_data_dir() {
        Ok(dir) => dir.join("user.db"),
        Err(_) => std::path::PathBuf::from("user.db"),
    };
    let user_conn = open_capture_db(Some(&user_db_path));
    if let Err(err) = service.as_ref() {
        eprintln!("shouci-mac: dictionary unavailable: {err}");
        eprintln!("shouci-mac: looked at {}", dictionary_path(None).display());
    }
    if let Err(err) = user_conn.as_ref() {
        eprintln!("shouci-mac: user db unavailable: {err}");
    }

    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let status_bar = NSStatusBar::systemStatusBar();
    let item = status_bar.statusItemWithLength(NSVariableStatusItemLength);
    let button = item.button(mtm).expect("status item must have a button");
    button.setTitle(ns_string!("文"));

    // The controller owns the popover and all its views. Both come up only
    // when the core services did; otherwise there is nothing to capture to.
    let delegate = match (service, user_conn) {
        (Ok(service), Ok(user_conn)) => Some(ui::install(mtm, button.clone(), service, user_conn)),
        (Err(err), _) | (_, Err(err)) => {
            eprintln!("shouci-mac: running menu-only: {err}");
            None
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
        // Degraded mode (no dictionary/database): a Quit-only menu so the
        // process is never stranded without UI.
        let menu = NSMenu::new(mtm);
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
    }

    debuglog::event("started");
    app.run();
}
