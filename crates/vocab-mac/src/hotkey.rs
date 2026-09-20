//! System-wide hotkey via Carbon `RegisterEventHotKey`.
//!
//! This is the only place in the crate that talks to Carbon: `AppKit` (even
//! via `objc2`) exposes no global-hotkey API, and the alternatives are worse
//! (`CGEventTap` needs an Accessibility permission prompt; menu key
//! equivalents only fire while the menu is open). Carbon hotkeys need no
//! permission and are delivered on the main run loop, so the callback may do
//! `AppKit` work directly.
//!
//! # Safety contract
//!
//! - The `extern` block mirrors `HIToolbox/CarbonEvents.h`; layouts were
//!   verified against the macOS 26 SDK (`EventHotKeyID`, `EventTypeSpec`).
//! - The C callback never touches Rust state except through `CALLBACK`, a
//!   `Mutex`-guarded slot written once at registration.
//! - `Registration` unregisters on drop; the process exit path drops it via
//!   the owner in `main`.

use std::ffi::c_void;
use std::fmt;
use std::sync::Mutex;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn GetApplicationEventTarget() -> EventTargetRef;
    fn InstallEventHandler(
        target: EventTargetRef,
        handler: EventHandlerProc,
        num_types: u32,
        types: *const EventTypeSpec,
        user_data: *mut c_void,
        out_handler: *mut EventHandlerRef,
    ) -> OSStatus;
    fn RegisterEventHotKey(
        key_code: u32,
        modifiers: u32,
        hotkey_id: EventHotKeyID,
        target: EventTargetRef,
        options: u32,
        out_ref: *mut EventHotKeyRef,
    ) -> OSStatus;
    fn UnregisterEventHotKey(hotkey: EventHotKeyRef) -> OSStatus;
}

type OSStatus = i32;
type EventTargetRef = *mut c_void;
type EventHandlerCallRef = *mut c_void;
type EventRef = *mut c_void;
type EventHandlerRef = *mut c_void;
type EventHotKeyRef = *mut c_void;
type EventHandlerProc =
    unsafe extern "C" fn(EventHandlerCallRef, EventRef, *mut c_void) -> OSStatus;

/// `kEventClassKeyboard` (`'keyb'`) and `kEventHotKeyPressed` (`5`).
const HOTKEY_EVENT: EventTypeSpec = EventTypeSpec {
    event_class: 0x6B65_7962,
    event_kind: 5,
};

/// Default binding: Ctrl+Opt+V (`kVK_ANSI_V` = `0x09`).
pub const DEFAULT_KEY_CODE: u32 = 0x09;
/// `controlKey | optionKey` (`(1 << 12) | (1 << 11)`).
pub const DEFAULT_MODIFIERS: u32 = (1 << 12) | (1 << 11);

#[repr(C)]
#[derive(Clone, Copy)]
struct EventTypeSpec {
    event_class: u32,
    event_kind: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct EventHotKeyID {
    signature: u32,
    id: u32,
}

/// `0x56434252` (`"VCBR"`).
const HOTKEY_SIGNATURE: u32 = 0x5643_4252;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotKeyError(pub OSStatus);

impl fmt::Display for HotKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "hotkey registration failed (OSStatus {})", self.0)
    }
}

impl std::error::Error for HotKeyError {}

static CALLBACK: Mutex<Option<Box<dyn Fn() + Send>>> = Mutex::new(None);

/// Carbon event callback. Runs on the main run loop by framework contract.
unsafe extern "C" fn handler(
    _call: EventHandlerCallRef,
    _event: EventRef,
    _data: *mut c_void,
) -> OSStatus {
    if let Ok(slot) = CALLBACK.lock().as_deref()
        && let Some(callback) = slot
    {
        callback();
    }
    0
}

/// Registered hotkey. Unregisters on drop.
pub struct Registration {
    hotkey: EventHotKeyRef,
}

impl Drop for Registration {
    fn drop(&mut self) {
        // SAFETY: `hotkey` came from a successful `RegisterEventHotKey`.
        unsafe {
            UnregisterEventHotKey(self.hotkey);
        }
    }
}

/// Registers a system-wide hotkey invoking `callback` on the main thread.
///
/// # Errors
///
/// Returns [`HotKeyError`] if the handler or key registration fails, or if
/// the callback slot is already taken (register once per process).
///
/// # Panics
///
/// Panics if the callback slot mutex is poisoned, which cannot happen
/// without a prior panic inside a callback.
///
/// # Safety
///
/// The caller must ensure the process pumps the main run loop (it does:
/// `NSApplication::run`). The callback must be main-thread-safe; Carbon
/// delivers hotkey events on the main loop.
pub unsafe fn register(
    key_code: u32,
    modifiers: u32,
    callback: impl Fn() + Send + 'static,
) -> Result<Registration, HotKeyError> {
    {
        let mut slot = CALLBACK.lock().expect("hotkey callback slot poisoned");
        if slot.is_some() {
            return Err(HotKeyError(-1));
        }
        *slot = Some(Box::new(callback));
    }
    // SAFETY: layouts mirror the SDK headers (see module docs); the handler
    // is a plain non-capturing `extern "C"` fn; `out` pointers are valid
    // stack slots for the duration of each call.
    unsafe {
        let target = GetApplicationEventTarget();
        let mut handler_ref: EventHandlerRef = std::ptr::null_mut();
        let status = InstallEventHandler(
            target,
            handler,
            1,
            &HOTKEY_EVENT,
            std::ptr::null_mut(),
            &raw mut handler_ref,
        );
        if status != 0 {
            return Err(HotKeyError(status));
        }
        let id = EventHotKeyID {
            signature: HOTKEY_SIGNATURE,
            id: 1,
        };
        let mut hotkey: EventHotKeyRef = std::ptr::null_mut();
        let status = RegisterEventHotKey(key_code, modifiers, id, target, 0, &raw mut hotkey);
        if status != 0 {
            return Err(HotKeyError(status));
        }
        Ok(Registration { hotkey })
    }
}
