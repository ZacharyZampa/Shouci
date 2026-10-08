import AppKit
import SwiftUI

/// A borderless panel that takes typing without activating Shouci: the app
/// you were in stays in front, and gets the keyboard back when the panel
/// closes.
final class QuickSearchPanel: NSPanel {
    /// ⌘Z with nothing typed: take back the last save. True when it did.
    var undoSave: () -> Bool = { false }
    var openSettings: () -> Void = {}
    var openLibrary: () -> Void = {}

    init() {
        super.init(
            contentRect: NSRect(x: 0, y: 0, width: 440, height: 300),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered, defer: true)
        isFloatingPanel = true
        level = .statusBar
        collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .transient, .ignoresCycle]
        isOpaque = false
        backgroundColor = .clear
        hasShadow = true
        hidesOnDeactivate = false
        isMovable = false
        animationBehavior = .utilityWindow
        setAccessibilityTitle("Shouci quick search")
    }

    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }

    /// Edit shortcuts handled here as well as in the main menu: while the
    /// panel has the keyboard, Shouci is not the active app.
    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        if super.performKeyEquivalent(with: event) { return true }
        let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        guard let key = event.charactersIgnoringModifiers?.lowercased() else { return false }
        let action: Selector
        switch (flags, key) {
        case ([.command], "z"):
            if undoSave() { return true }
            action = Selector(("undo:"))
        case ([.command, .shift], "z"): action = Selector(("redo:"))
        case ([.command], "x"): action = #selector(NSText.cut(_:))
        case ([.command], "c"): action = #selector(NSText.copy(_:))
        case ([.command], "v"): action = #selector(NSText.paste(_:))
        case ([.command], "a"): action = #selector(NSText.selectAll(_:))
        case ([.command], ","):
            openSettings()
            return true
        case ([.command], "o"):
            openLibrary()
            return true
        default:
            return false
        }
        return NSApp.sendAction(action, to: nil, from: self)
    }
}

/// Shows, places, sizes, and hides the panel.
@MainActor
final class QuickSearchController: NSObject, NSWindowDelegate {
    let model: QuickSearch
    private let panel = QuickSearchPanel()
    /// Where the menu-bar icon is on screen, when it is shown.
    var anchor: () -> NSRect? = { nil }
    var visibilityChanged: (Bool) -> Void = { _ in }
    private var hiddenAt = Date.distantPast

    init(model: QuickSearch, openSettings: @escaping () -> Void) {
        self.model = model
        super.init()
        // Resizing the panel inside SwiftUI's layout pass would lay it out
        // again from within itself; resize just after instead.
        let hosting = NSHostingView(
            rootView: QuickSearchView(model: model) { [weak self] height in
                Task { @MainActor in self?.fit(height: height) }
            })
        hosting.sizingOptions = []
        panel.contentView = hosting
        panel.delegate = self
        panel.undoSave = { [weak model] in model?.hasQuery == false && model?.undo() == true }
        panel.openSettings = openSettings
        panel.openLibrary = { [weak model] in model?.openLibrary(nil) }
        model.close = { [weak self] in self?.hide() }
    }

    var isVisible: Bool { panel.isVisible }

    func toggle() {
        if panel.isKeyWindow {
            hide()
        } else if panel.isVisible {
            panel.makeKey()
        } else if Date.now.timeIntervalSince(hiddenAt) > 0.3 {
            // A click on 文 while the panel is open first takes the keyboard
            // from it, which hides it; that click should not reopen it.
            show()
        }
    }

    func show() {
        model.opened()
        place()
        // Ready for typing the moment the panel appears: keys pressed right
        // after the shortcut must land in an empty, focused field, not wait
        // for SwiftUI's next update.
        panel.contentView?.layoutSubtreeIfNeeded()
        let field = panel.contentView.flatMap(Self.textField)
        field?.stringValue = ""
        panel.makeKeyAndOrderFront(nil)
        if let field, field.currentEditor() == nil { panel.makeFirstResponder(field) }
        visibilityChanged(true)
    }

    private static func textField(in view: NSView) -> NSTextField? {
        if let field = view as? NSTextField, field.isEditable { return field }
        for subview in view.subviews {
            if let field = textField(in: subview) { return field }
        }
        return nil
    }

    func hide() {
        guard panel.isVisible else { return }
        panel.orderOut(nil)
        hiddenAt = .now
        model.closed()
        visibilityChanged(false)
    }

    func windowDidResignKey(_ notification: Notification) {
        hide()
    }

    /// Under 文 when it is in the menu bar; otherwise centered near the top
    /// of the screen with the pointer. Results are kept to what fits between
    /// there and the foot of the screen.
    private func place() {
        let size = panel.frame.size
        let top: NSPoint
        let visible: NSRect
        if let anchor = anchor(),
            let screen = NSScreen.screens.first(where: { $0.frame.intersects(anchor) })
        {
            visible = screen.visibleFrame
            let x = min(max(anchor.midX - size.width / 2, visible.minX + 8), visible.maxX - size.width - 8)
            top = NSPoint(x: x, y: anchor.minY - 8)
        } else {
            let pointer = NSEvent.mouseLocation
            let screen = NSScreen.screens.first { $0.frame.contains(pointer) } ?? NSScreen.main
            guard let frame = screen?.visibleFrame else { return }
            visible = frame
            top = NSPoint(x: visible.midX - size.width / 2, y: visible.maxY - visible.height * 0.18)
        }
        panel.setFrameTopLeftPoint(top)
        model.fit(height: top.y - visible.minY - 8)
    }

    /// Follows the content's height, keeping the top edge where it is.
    private func fit(height: CGFloat) {
        let height = ceil(height)
        guard height > 0, abs(panel.frame.height - height) > 0.5 else { return }
        var frame = panel.frame
        frame.origin.y = frame.maxY - height
        frame.size.height = height
        panel.setFrame(frame, display: true)
        panel.invalidateShadow()
    }
}
