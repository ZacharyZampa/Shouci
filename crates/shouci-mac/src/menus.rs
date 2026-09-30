//! Main menu. An accessory app never shows a menu bar, but `AppKit` still
//! routes key equivalents through the main menu: without an Edit menu,
//! ⌘V / ⌘C / ⌘A / ⌘Z do nothing in the capture field. The Word menu gives
//! every row action a keyboard shortcut.

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{MainThreadMarker, sel};
use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSMenu, NSMenuItem};
use objc2_foundation::NSString;

use crate::ui::CaptureDelegate;

/// ⌥⌘⌫ key character (`NSBackspaceCharacter`). Finder uses the same combo
/// for "Delete Immediately"; plain ⌘⌫ stays with the text field, where it
/// deletes to the start of the line.
pub(crate) const DELETE_KEY: &str = "\u{8}";

struct Entry<'a> {
    title: &'a str,
    action: Sel,
    key: &'a str,
    modifiers: NSEventModifierFlags,
    /// `None` sends the action up the responder chain (the field editor,
    /// the key window, then the app).
    target: Option<&'a AnyObject>,
}

fn entry<'a>(
    title: &'a str,
    action: Sel,
    key: &'a str,
    modifiers: NSEventModifierFlags,
) -> Entry<'a> {
    Entry {
        title,
        action,
        key,
        modifiers,
        target: None,
    }
}

fn submenu(mtm: MainThreadMarker, bar: &NSMenu, title: &str, entries: &[Option<Entry<'_>>]) {
    let menu = NSMenu::initWithTitle(mtm.alloc(), &NSString::from_str(title));
    for entry in entries {
        let Some(entry) = entry else {
            menu.addItem(&NSMenuItem::separatorItem(mtm));
            continue;
        };
        // SAFETY: standard initializer; the item is retained by the menu.
        let item: Retained<NSMenuItem> = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                mtm.alloc(),
                &NSString::from_str(entry.title),
                Some(entry.action),
                &NSString::from_str(entry.key),
            )
        };
        item.setKeyEquivalentModifierMask(entry.modifiers);
        // SAFETY: the target (the capture delegate) lives for the process
        // lifetime; `None` means the responder chain.
        unsafe {
            item.setTarget(entry.target);
        }
        menu.addItem(&item);
    }
    let holder = NSMenuItem::new(mtm);
    holder.setSubmenu(Some(&menu));
    bar.addItem(&holder);
}

/// Installs the main menu. Without a delegate (degraded mode) only the
/// app, Edit, and Window menus exist.
pub(crate) fn install(mtm: MainThreadMarker, delegate: Option<&CaptureDelegate>) {
    let command = NSEventModifierFlags::Command;
    let shift = NSEventModifierFlags::Shift;
    let target: Option<&AnyObject> = delegate.map(|delegate| {
        let object: &AnyObject = delegate;
        object
    });
    let bar = NSMenu::new(mtm);

    let mut app_entries = Vec::new();
    if target.is_some() {
        app_entries.push(Some(Entry {
            target,
            ..entry("Settings…", sel!(showSettings:), ",", command)
        }));
        app_entries.push(None);
    }
    app_entries.push(Some(entry("Quit Shouci", sel!(terminate:), "q", command)));
    submenu(mtm, &bar, "Shouci", &app_entries);

    submenu(
        mtm,
        &bar,
        "Edit",
        &[
            Some(entry("Undo", sel!(undo:), "z", command)),
            Some(entry("Redo", sel!(redo:), "z", command | shift)),
            None,
            Some(entry("Cut", sel!(cut:), "x", command)),
            Some(entry("Copy", sel!(copy:), "c", command)),
            Some(entry("Paste", sel!(paste:), "v", command)),
            Some(entry("Select All", sel!(selectAll:), "a", command)),
        ],
    );

    if target.is_some() {
        submenu(
            mtm,
            &bar,
            "Word",
            &[
                Some(Entry {
                    target,
                    ..entry(
                        "Mark as Needs Review",
                        sel!(markReviewSelected:),
                        "r",
                        command,
                    )
                }),
                Some(Entry {
                    target,
                    ..entry(
                        "Archive",
                        sel!(archiveSelected:),
                        "a",
                        command | NSEventModifierFlags::Control,
                    )
                }),
                None,
                Some(Entry {
                    target,
                    ..entry(
                        "Delete…",
                        sel!(deleteSelected:),
                        DELETE_KEY,
                        command | NSEventModifierFlags::Option,
                    )
                }),
            ],
        );
    }

    submenu(
        mtm,
        &bar,
        "Window",
        &[Some(entry("Close", sel!(performClose:), "w", command))],
    );

    NSApplication::sharedApplication(mtm).setMainMenu(Some(&bar));
}
