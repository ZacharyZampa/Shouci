import AppKit

/// 文 in the menu bar. A click toggles quick search; a right-click (or
/// Control-click) shows the menu.
@MainActor
final class StatusItemController: NSObject {
    private let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    private let menu: NSMenu
    private let onClick: () -> Void

    init(menu: NSMenu, onClick: @escaping () -> Void) {
        self.menu = menu
        self.onClick = onClick
        super.init()
        guard let button = item.button else { return }
        button.title = "文"
        button.font = .systemFont(ofSize: 15, weight: .medium)
        // VoiceOver would otherwise read the glyph as "wén".
        button.setAccessibilityLabel("Shouci")
        button.target = self
        button.action = #selector(clicked(_:))
        button.sendAction(on: [.leftMouseUp, .rightMouseUp])
    }

    var isVisible: Bool {
        get { item.isVisible }
        set { item.isVisible = newValue }
    }

    /// The icon's place on screen, when it is in the menu bar.
    var anchor: NSRect? {
        guard item.isVisible, let button = item.button, let window = button.window else { return nil }
        return window.convertToScreen(button.convert(button.bounds, to: nil))
    }

    func setHighlighted(_ highlighted: Bool) {
        item.button?.highlight(highlighted)
    }

    func setShortcut(_ display: String) {
        item.button?.toolTip = "Shouci — \(display) to search or add a word"
    }

    @objc private func clicked(_ sender: NSStatusBarButton) {
        let event = NSApp.currentEvent
        if event?.type == .rightMouseUp || event?.modifierFlags.contains(.control) == true {
            menu.popUp(positioning: nil, at: NSPoint(x: 0, y: sender.bounds.height + 5), in: sender)
        } else {
            onClick()
        }
    }
}
