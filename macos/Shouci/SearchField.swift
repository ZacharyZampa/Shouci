import AppKit
import SwiftUI

/// The search fields of the panel and the library window. AppKit fields
/// rather than a SwiftUI `TextField`: arrows, Return, and Esc arrive as commands only after the
/// input method passes on them, so choosing a pinyin IME candidate never
/// moves the selection or saves a word. Text being composed is not searched.
struct SearchField: NSViewRepresentable {
    /// The panel's large borderless field, or a toolbar search field.
    enum Look { case panel, toolbar }

    @Binding var text: String
    var look: Look = .panel
    var focusRequests: Int
    var onMove: (Int) -> Void
    var onSubmit: () -> Void
    var onEscape: () -> Void

    func makeNSView(context: Context) -> NSTextField {
        let field: NSTextField
        switch look {
        case .panel:
            field = NSTextField()
            field.isBordered = false
            field.drawsBackground = false
            field.focusRingType = .none
            field.font = .systemFont(ofSize: 18)
            field.textColor = .labelColor
        case .toolbar:
            let search = NSSearchField()
            search.sendsSearchStringImmediately = true
            search.target = context.coordinator
            search.action = #selector(Coordinator.searched(_:))
            field = search
        }
        field.placeholderString = "Search or add a word"
        field.setAccessibilityLabel("Search or add a word")
        field.cell?.usesSingleLineMode = true
        field.cell?.lineBreakMode = .byTruncatingTail
        field.delegate = context.coordinator
        return field
    }

    func updateNSView(_ field: NSTextField, context: Context) {
        context.coordinator.parent = self
        let composing = (field.currentEditor() as? NSTextView)?.hasMarkedText() ?? false
        if field.stringValue != text && !composing {
            field.stringValue = text
        }
        if context.coordinator.focusRequests != focusRequests {
            context.coordinator.focusRequests = focusRequests
            // Focusing a field selects its text, so a field already being
            // typed in is left alone: re-focusing it would select what was
            // just typed and the next key would replace it.
            if field.window != nil {
                if field.currentEditor() == nil { field.window?.makeFirstResponder(field) }
            } else {
                DispatchQueue.main.async {
                    if field.currentEditor() == nil { field.window?.makeFirstResponder(field) }
                }
            }
        }
    }

    /// Starts at the current request count: a field SwiftUI rebuilds must
    /// not take the keyboard (and select its text) on its own.
    func makeCoordinator() -> Coordinator {
        let coordinator = Coordinator(parent: self)
        coordinator.focusRequests = focusRequests
        return coordinator
    }

    @MainActor
    final class Coordinator: NSObject, NSSearchFieldDelegate {
        var parent: SearchField
        var focusRequests = -1

        init(parent: SearchField) {
            self.parent = parent
        }

        /// The search field's clear button.
        @objc func searched(_ field: NSSearchField) {
            if let editor = field.currentEditor() as? NSTextView, editor.hasMarkedText() { return }
            parent.text = field.stringValue
        }

        func controlTextDidChange(_ notification: Notification) {
            guard let field = notification.object as? NSTextField else { return }
            if let editor = field.currentEditor() as? NSTextView, editor.hasMarkedText() { return }
            parent.text = field.stringValue
        }

        func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
            switch selector {
            case #selector(NSResponder.moveUp(_:)):
                parent.onMove(-1)
            case #selector(NSResponder.moveDown(_:)):
                parent.onMove(1)
            case #selector(NSResponder.insertNewline(_:)):
                parent.onSubmit()
            case #selector(NSResponder.cancelOperation(_:)):
                parent.onEscape()
            default:
                return false
            }
            return true
        }
    }
}
