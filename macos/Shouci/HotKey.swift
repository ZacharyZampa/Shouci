import AppKit
import Carbon.HIToolbox

/// A global shortcut: a Carbon key code and Carbon modifier bits.
struct HotKeyBinding: Equatable, Sendable {
    var keyCode: UInt32
    var modifiers: UInt32
    /// The key as printed: `V`, `Space`, `F5`.
    var key: String

    static let standard = HotKeyBinding(
        keyCode: UInt32(kVK_ANSI_V), modifiers: UInt32(controlKey | optionKey), key: "V")

    /// Modifier symbols in the standard order, then the key: `⌃⌥V`.
    var symbols: [String] {
        let order: [(Int, String)] = [(controlKey, "⌃"), (optionKey, "⌥"), (shiftKey, "⇧"), (cmdKey, "⌘")]
        return order.filter { modifiers & UInt32($0.0) != 0 }.map(\.1) + [key]
    }

    var display: String { symbols.joined() }

    /// Control or Command, or a function key. macOS refuses bare keys and
    /// Option-only (or Option+Shift) combinations, and a Shift-only shortcut
    /// would take a capital letter from every app.
    var isValid: Bool {
        if Self.functionKeys.contains(Int(keyCode)) { return true }
        return modifiers & UInt32(controlKey | cmdKey) != 0
    }

    private static let functionKeys: Set<Int> = [
        kVK_F1, kVK_F2, kVK_F3, kVK_F4, kVK_F5, kVK_F6, kVK_F7, kVK_F8, kVK_F9, kVK_F10,
        kVK_F11, kVK_F12, kVK_F13, kVK_F14, kVK_F15, kVK_F16, kVK_F17, kVK_F18, kVK_F19, kVK_F20,
    ]

    /// The shortcut a key-down event asks for.
    init?(event: NSEvent) {
        let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        var modifiers = 0
        if flags.contains(.control) { modifiers |= controlKey }
        if flags.contains(.option) { modifiers |= optionKey }
        if flags.contains(.shift) { modifiers |= shiftKey }
        if flags.contains(.command) { modifiers |= cmdKey }
        let code = Int(event.keyCode)
        let key = Self.names[code] ?? event.charactersIgnoringModifiers?.uppercased() ?? ""
        guard !key.isEmpty, !key.contains(where: \.isNewline) else { return nil }
        self.init(keyCode: UInt32(code), modifiers: UInt32(modifiers), key: key)
    }

    init(keyCode: UInt32, modifiers: UInt32, key: String) {
        self.keyCode = keyCode
        self.modifiers = modifiers
        self.key = key
    }

    private static let names: [Int: String] = [
        kVK_Space: "Space", kVK_Return: "↩", kVK_Tab: "⇥", kVK_Delete: "⌫", kVK_ForwardDelete: "⌦",
        kVK_LeftArrow: "←", kVK_RightArrow: "→", kVK_UpArrow: "↑", kVK_DownArrow: "↓",
        kVK_Home: "↖", kVK_End: "↘", kVK_PageUp: "⇞", kVK_PageDown: "⇟",
        kVK_F1: "F1", kVK_F2: "F2", kVK_F3: "F3", kVK_F4: "F4", kVK_F5: "F5", kVK_F6: "F6",
        kVK_F7: "F7", kVK_F8: "F8", kVK_F9: "F9", kVK_F10: "F10", kVK_F11: "F11", kVK_F12: "F12",
    ]

    // The keys the menu-bar app has always used, so a shortcut set there
    // carries over.
    private enum Keys {
        static let code = "ShouciHotkeyKeyCode"
        static let modifiers = "ShouciHotkeyModifiers"
        static let display = "ShouciHotkeyDisplay"
    }

    static func load(from defaults: UserDefaults) -> HotKeyBinding {
        guard defaults.object(forKey: Keys.code) != nil,
            let code = UInt32(exactly: defaults.integer(forKey: Keys.code)),
            let modifiers = UInt32(exactly: defaults.integer(forKey: Keys.modifiers))
        else { return .standard }
        let display = defaults.string(forKey: Keys.display) ?? ""
        let key = String(display.drop(while: { "⌃⌥⇧⌘".contains($0) }))
        let binding = HotKeyBinding(keyCode: code, modifiers: modifiers, key: key.isEmpty ? "?" : key)
        return binding.isValid ? binding : .standard
    }

    func store(in defaults: UserDefaults) {
        defaults.set(Int(keyCode), forKey: Keys.code)
        defaults.set(Int(modifiers), forKey: Keys.modifiers)
        defaults.set(display, forKey: Keys.display)
    }
}

struct HotKeyError: LocalizedError {
    let status: OSStatus

    var errorDescription: String? {
        if status == OSStatus(eventHotKeyExistsErr) {
            return "Another app already uses this shortcut."
        }
        return "The shortcut could not be set (error \(status))."
    }
}

/// A system-wide shortcut through Carbon `RegisterEventHotKey`: no
/// Accessibility permission, and events arrive on the main run loop.
@MainActor
final class HotKey {
    private var hotKeyRef: EventHotKeyRef?
    private var handlerRef: EventHandlerRef?
    private var action: () -> Void = {}

    /// Carbon calls back through a C function, which can't capture; one
    /// shortcut per app, so it finds its way back through here.
    private static weak var current: HotKey?

    func register(_ binding: HotKeyBinding, action: @escaping () -> Void) throws {
        try installHandler()
        unregister()
        var ref: EventHotKeyRef?
        let id = EventHotKeyID(signature: OSType(0x5643_4252), id: 1)
        let status = RegisterEventHotKey(
            binding.keyCode, binding.modifiers, id, GetApplicationEventTarget(), 0, &ref)
        guard status == noErr, let ref else { throw HotKeyError(status: status) }
        hotKeyRef = ref
        self.action = action
        Self.current = self
    }

    func unregister() {
        if let hotKeyRef {
            UnregisterEventHotKey(hotKeyRef)
            self.hotKeyRef = nil
        }
    }

    private func installHandler() throws {
        guard handlerRef == nil else { return }
        var spec = EventTypeSpec(
            eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
        let status = InstallEventHandler(
            GetApplicationEventTarget(),
            { _, _, _ in
                MainActor.assumeIsolated { HotKey.current?.action() }
                return noErr
            },
            1, &spec, nil, &handlerRef)
        guard status == noErr else { throw HotKeyError(status: status) }
    }
}
