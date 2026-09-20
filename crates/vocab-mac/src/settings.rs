//! Hotkey recorder view + binding persistence.
//!
//! The Settings window's recorder is a tiny `define_class!` view: click to
//! arm, press the new shortcut, done. Bindings persist in `UserDefaults`;
//! the actual (un)registration lives on the delegate in `ui`, which owns the
//! Carbon guard.

use std::cell::Cell;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSTextField, NSView};
use objc2_foundation::{NSRect, NSString, NSUserDefaults, ns_string};

use crate::hotkey;

const DEFAULTS_CODE_KEY: &str = "VocabBarHotkeyKeyCode";
const DEFAULTS_MODS_KEY: &str = "VocabBarHotkeyModifiers";
const DEFAULTS_DISPLAY_KEY: &str = "VocabBarHotkeyDisplay";

/// A hotkey binding: Carbon keycode + Carbon modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HotkeyBinding {
    pub keycode: u32,
    pub modifiers: u32,
}

impl HotkeyBinding {
    fn defaults() -> Self {
        Self {
            keycode: hotkey::DEFAULT_KEY_CODE,
            modifiers: hotkey::DEFAULT_MODIFIERS,
        }
    }
}

/// Loads the stored binding, falling back to the default when unset or
/// corrupt.
pub(crate) fn load_binding() -> HotkeyBinding {
    let defaults = NSUserDefaults::standardUserDefaults();
    let code = u32::try_from(defaults.integerForKey(ns_string!(DEFAULTS_CODE_KEY))).ok();
    let modifiers = u32::try_from(defaults.integerForKey(ns_string!(DEFAULTS_MODS_KEY))).ok();
    match (code, modifiers) {
        (Some(keycode), Some(modifiers)) if valid_hotkey(modifiers) => {
            HotkeyBinding { keycode, modifiers }
        }
        _ => HotkeyBinding::defaults(),
    }
}

/// Persists a binding (code, modifiers, and its display text together).
pub(crate) fn store_binding(binding: HotkeyBinding, display: &str) {
    let defaults = NSUserDefaults::standardUserDefaults();
    defaults.setInteger_forKey(
        objc2::ffi::NSInteger::try_from(binding.keycode).unwrap_or(0),
        ns_string!(DEFAULTS_CODE_KEY),
    );
    defaults.setInteger_forKey(
        objc2::ffi::NSInteger::try_from(binding.modifiers).unwrap_or(0),
        ns_string!(DEFAULTS_MODS_KEY),
    );
    let display_value = NSString::from_str(display);
    let display_object: &AnyObject = &display_value;
    // SAFETY: storing an `NSString` under our own key; property-list safe.
    unsafe {
        defaults.setObject_forKey(Some(display_object), ns_string!(DEFAULTS_DISPLAY_KEY));
    }
}

/// Display text for the recorder: stored text, or the default combo.
pub(crate) fn display_string() -> String {
    let defaults = NSUserDefaults::standardUserDefaults();
    if let Some(text) = defaults.stringForKey(ns_string!(DEFAULTS_DISPLAY_KEY)) {
        return text.to_string();
    }
    String::from("⌃⌥V")
}

/// Bare keys and Option-only (or Option+Shift) combos fail to register
/// on macOS, so refuse them up front.
pub(crate) fn valid_hotkey(modifiers: u32) -> bool {
    const CONTROL: u32 = 1 << 12;
    const OPTION: u32 = 1 << 11;
    const COMMAND: u32 = 1 << 8;
    const SHIFT: u32 = 1 << 9;
    if modifiers == 0 {
        return false;
    }
    let meaningful = modifiers & (CONTROL | COMMAND | SHIFT);
    if meaningful == 0 {
        return false;
    }
    // Option+Shift alone (no Ctrl/Cmd) is in the same broken family.
    if modifiers & OPTION != 0 && meaningful == SHIFT {
        return false;
    }
    true
}

/// Maps Cocoa modifier flags to Carbon modifier bits.
pub(crate) fn carbon_modifiers(flags: NSEventModifierFlags) -> u32 {
    let mut modifiers = 0;
    if flags.contains(NSEventModifierFlags::Control) {
        modifiers |= 1 << 12;
    }
    if flags.contains(NSEventModifierFlags::Option) {
        modifiers |= 1 << 11;
    }
    if flags.contains(NSEventModifierFlags::Command) {
        modifiers |= 1 << 8;
    }
    if flags.contains(NSEventModifierFlags::Shift) {
        modifiers |= 1 << 9;
    }
    modifiers
}

/// Display text for a fresh capture: modifier glyphs + the key character.
pub(crate) fn display_for_event(event: &NSEvent) -> String {
    let flags = event.modifierFlags();
    let mut text = String::new();
    if flags.contains(NSEventModifierFlags::Control) {
        text.push('⌃');
    }
    if flags.contains(NSEventModifierFlags::Option) {
        text.push('⌥');
    }
    if flags.contains(NSEventModifierFlags::Shift) {
        text.push('⇧');
    }
    if flags.contains(NSEventModifierFlags::Command) {
        text.push('⌘');
    }
    if let Some(characters) = event.charactersIgnoringModifiers() {
        text.push_str(&characters.to_string().to_uppercase());
    } else {
        use std::fmt::Write as _;
        let _ignored = write!(text, "keycode {}", event.keyCode());
    }
    text
}

// `pub(crate)` because the `pub(crate)` recorder class names it in
// `#[ivars = …]`; fields stay module-private, accessed via `ivars()`.
pub(crate) struct RecorderIvars {
    label: Retained<NSTextField>,
    recording: Cell<bool>,
}

define_class!(
    /// Click-to-arm hotkey recorder. See module docs.
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[ivars = RecorderIvars]
    pub(crate) struct HotkeyRecorder;

    // SAFETY: `NSObjectProtocol` has no safety requirements.
    unsafe impl NSObjectProtocol for HotkeyRecorder {}

    impl HotkeyRecorder {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _event: &NSEvent) {
            self.ivars().recording.set(true);
            self.refresh();
            if let Some(window) = self.window() {
                window.makeFirstResponder(Some(self));
            }
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            if !self.ivars().recording.get() {
                return;
            }
            // Plain Esc cancels the recording.
            if event.keyCode() == 53 && carbon_modifiers(event.modifierFlags()) == 0 {
                self.ivars().recording.set(false);
                self.refresh();
                return;
            }
            let binding = HotkeyBinding {
                keycode: u32::from(event.keyCode()),
                modifiers: carbon_modifiers(event.modifierFlags()),
            };
            if !valid_hotkey(binding.modifiers) {
                self.ivars().label.setStringValue(ns_string!(
                    "Need Ctrl, Cmd, or Shift — Esc cancels"
                ));
                return;
            }
            self.ivars().recording.set(false);
            crate::ui::apply_recorded_hotkey(binding, &display_for_event(event));
            self.refresh();
        }

        #[unsafe(method(resignFirstResponder))]
        fn resign_first_responder(&self) -> bool {
            self.ivars().recording.set(false);
            self.refresh();
            true
        }
    }
);

impl HotkeyRecorder {
    fn refresh(&self) {
        let ivars = self.ivars();
        if ivars.recording.get() {
            ivars
                .label
                .setStringValue(ns_string!("Press shortcut… (Esc cancels)"));
        } else {
            ivars
                .label
                .setStringValue(&NSString::from_str(&display_string()));
        }
    }

    /// Re-reads the stored binding (used after an external change).
    pub(crate) fn refresh_display(&self) {
        self.ivars().recording.set(false);
        self.refresh();
    }
}

/// Builds the recorder view (fixed frame; the caller positions it).
pub(crate) fn recorder_view(mtm: MainThreadMarker, frame: NSRect) -> Retained<HotkeyRecorder> {
    let label = NSTextField::labelWithString(ns_string!(""), mtm);
    label.setFrame(NSRect::new(
        objc2_foundation::NSPoint::new(8.0, 4.0),
        objc2_foundation::NSSize::new(frame.size.width - 16.0, 20.0),
    ));
    label.setFont(Some(&objc2_app_kit::NSFont::systemFontOfSize(13.0)));
    label.setAlignment(objc2_app_kit::NSTextAlignment::Center);
    let this = HotkeyRecorder::alloc(mtm).set_ivars(RecorderIvars {
        label: label.clone(),
        recording: Cell::new(false),
    });
    // SAFETY: `NSObject init` on fresh allocation; matches `define_class!`.
    let this: Retained<HotkeyRecorder> = unsafe { msg_send![super(this), init] };
    this.addSubview(&label);
    this.setWantsLayer(true);
    this.refresh_display();
    this
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags(mods: u32) -> NSEventModifierFlags {
        // Carbon bits under test: Ctrl 1<<12, Opt 1<<11, Cmd 1<<8, Shift 1<<9.
        let mut flags = NSEventModifierFlags::empty();
        if mods & (1 << 12) != 0 {
            flags |= NSEventModifierFlags::Control;
        }
        if mods & (1 << 11) != 0 {
            flags |= NSEventModifierFlags::Option;
        }
        if mods & (1 << 8) != 0 {
            flags |= NSEventModifierFlags::Command;
        }
        if mods & (1 << 9) != 0 {
            flags |= NSEventModifierFlags::Shift;
        }
        flags
    }

    #[test]
    fn cocoa_flags_map_to_carbon_bits() {
        assert_eq!(
            carbon_modifiers(flags((1 << 12) | (1 << 11))),
            (1 << 12) | (1 << 11)
        );
        assert_eq!(carbon_modifiers(flags(1 << 8)), 1 << 8);
        assert_eq!(carbon_modifiers(flags(0)), 0);
    }

    #[test]
    fn validation_rejects_bare_and_option_only() {
        assert!(valid_hotkey((1 << 12) | (1 << 11))); // Ctrl+Opt+V default
        assert!(valid_hotkey(1 << 8)); // Cmd alone has a non-Option modifier
        assert!(!valid_hotkey(0)); // bare keys never register
        assert!(!valid_hotkey(1 << 11)); // Option-only fails on macOS
        assert!(!valid_hotkey((1 << 11) | (1 << 9))); // Option+Shift fails too
        assert!(valid_hotkey((1 << 11) | (1 << 12))); // Option is fine with Ctrl
    }

    #[test]
    fn round_trip_through_defaults() {
        // Preserve whatever the user had: a test run must never destroy it.
        let before = load_binding();
        let before_display = display_string();
        let binding = HotkeyBinding {
            keycode: 9,
            modifiers: (1 << 12) | (1 << 11),
        };
        store_binding(binding, "⌃⌥V");
        assert_eq!(load_binding(), binding);
        store_binding(before, &before_display);
        assert_eq!(load_binding(), before);
    }
}
