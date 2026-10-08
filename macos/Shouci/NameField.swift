import AppKit
import ShouciCore
import SwiftUI

/// Tags or collections: how the fields that name them read.
enum GroupKind {
    case tag
    case collection

    var noun: String {
        switch self {
        case .tag: "tag"
        case .collection: "collection"
        }
    }

    var icon: String {
        switch self {
        case .tag: "tag"
        case .collection: "folder"
        }
    }

    var chipStyle: Chip.Style {
        switch self {
        case .tag: .accent
        case .collection: .plain
        }
    }

    var prompt: String {
        switch self {
        case .tag: "Add tag"
        case .collection: "Add to collection"
        }
    }
}

/// What a typed name could mean: the names that already exist, and a new
/// one when nothing is called that.
struct NameSuggestions {
    enum Row: Hashable {
        /// A name in the library, with how many words it has.
        case existing(String, count: UInt64)
        /// A name the word already has: shown, so it isn't created again.
        case added(String)
        case create(String)

        var name: String {
            switch self {
            case .existing(let name, _), .added(let name), .create(let name): name
            }
        }

        var isChoosable: Bool {
            if case .added = self { false } else { true }
        }
    }

    private(set) var rows: [Row] = []
    /// The row ↵ picks before the arrows move: the first name that matches
    /// what was typed, or creating it when nothing does. Nil when the word
    /// already has the name typed, or every name that matches.
    private(set) var preferred: Int?

    /// `present` is what the word has now; `limit` caps the names that match
    /// what was typed. `recent` names, most recent first, lead the list
    /// before anything is typed, and ↵ picks the first of them.
    init(query: String, known: [GroupView], present: [String], recent: [String] = [], limit: Int = 6) {
        let typed = query.trimmingCharacters(in: .whitespaces)
        let has = { (name: String) in present.contains { $0.caseInsensitiveCompare(name) == .orderedSame } }

        guard !typed.isEmpty else {
            // Everything that exists, recent first, then most used, to
            // browse before typing.
            let others = known.filter { !has($0.name) }
            let lately = recent.compactMap { name in
                others.first { $0.name.caseInsensitiveCompare(name) == .orderedSame }
            }
            let rest = others.filter { group in !lately.contains { $0.name == group.name } }
                .sorted { $0.count > $1.count }
            rows = (lately + rest).map { .existing($0.name, count: $0.count) }
            preferred = lately.isEmpty ? nil : 0
            return
        }

        // Names on the word that the library doesn't have yet: added in the
        // editor and not saved.
        let unsaved = present.filter { name in !known.contains { $0.name.caseInsensitiveCompare(name) == .orderedSame } }
        let candidates = known.map { ($0.name, $0.count) } + unsaved.map { ($0, UInt64(0)) }
        let ranked: [(rank: Int, row: Row)] = candidates.compactMap { name, count in
            guard let found = name.range(of: typed, options: .caseInsensitive) else { return nil }
            let rank = found == name.startIndex..<name.endIndex ? 0 : found.lowerBound == name.startIndex ? 1 : 2
            let row: Row = has(name) ? .added(name) : .existing(name, count: count)
            return (rank, row)
        }
        let sorted = ranked.sorted {
            $0.rank != $1.rank ? $0.rank < $1.rank : $0.row.name.localizedStandardCompare($1.row.name) == .orderedAscending
        }
        let shown = sorted.prefix(limit)
        rows = shown.map(\.row)
        if shown.first?.rank == 0 {
            preferred = rows[0].isChoosable ? 0 : nil
        } else {
            rows.append(.create(typed))
            preferred = shown.isEmpty ? 0 : shown.firstIndex { $0.row.isChoosable }
        }
    }
}

/// Tags or collections as chips, each with a button that takes it off, then
/// a field that adds one.
struct NameList: View {
    let kind: GroupKind
    let names: [String]
    let known: [GroupView]
    let add: (String) -> Void
    let remove: (String) -> Void

    var body: some View {
        FlowLayout(spacing: 6) {
            ForEach(names, id: \.self) { name in
                Chip(text: name, style: kind.chipStyle) { remove(name) }
            }
            NameInput(kind: kind, known: known, present: names, choose: add)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .coordinateSpace(.named(NameInput.row))
    }
}

/// A field for one tag or collection. Typing lists the names that already
/// exist, with how many words each has, and offers to create the name when
/// it is new, so it is always clear which one ↵ picks.
struct NameInput: View {
    /// A menu under the field while it has the keyboard, or a list that
    /// stays below it (in a popover).
    enum Placement { case menu, list }

    /// The row the field sits in, which the menu stays inside.
    nonisolated static let row = "NameInput.row"

    let kind: GroupKind
    let known: [GroupView]
    let present: [String]
    var placement: Placement = .menu
    var focusOnAppear = false
    /// Names chosen lately, most recent first: shown first, and ↵ picks the
    /// first before anything is typed.
    var recent: [String] = []
    let choose: (String) -> Void
    var dismiss: (() -> Void)?
    /// What ⇥ does instead of moving to the next field.
    var tab: (() -> Void)?

    @State private var text = ""
    @State private var focused = false
    /// The row the arrows or the pointer moved to; nil, or a row a reload
    /// took away, follows `preferred`.
    @State private var moved: NameSuggestions.Row?
    /// The enclosing row, in the field's coordinates.
    @State private var row: CGRect = .null
    @State private var focusRequests = 0

    private var suggestions: NameSuggestions {
        NameSuggestions(query: text, known: known, present: present, recent: recent)
    }

    var body: some View {
        let suggestions = self.suggestions
        let highlighted = moved.flatMap(suggestions.rows.firstIndex(of:)) ?? suggestions.preferred
        switch placement {
        case .menu:
            // Under the field, moved left as far as it must be to stay in
            // the row, and no wider than the row.
            let width = row.isNull ? 260 : min(260, row.width)
            let x = row.isNull ? 0 : max(row.minX, min(0, row.maxX - width))
            field(suggestions, highlighted: highlighted)
                .frame(width: 150)
                .onGeometryChange(for: CGRect.self) { $0.bounds(of: .named(Self.row)) ?? .null } action: { row = $0 }
                .overlay(alignment: .topLeading) {
                    if focused && !suggestions.rows.isEmpty {
                        SuggestionRows(
                            kind: kind, rows: suggestions.rows, highlighted: highlighted,
                            hover: { hover($0, in: suggestions) }, pick: { pick(suggestions.rows[$0]) })
                        .padding(5)
                        .frame(width: width)
                        .background(
                            RoundedRectangle(cornerRadius: 9, style: .continuous)
                                .fill(Palette.list)
                                .strokeBorder(Palette.line))
                        .shadow(color: .black.opacity(0.16), radius: 10, y: 4)
                        .offset(x: x, y: 26)
                    }
                }
        case .list:
            VStack(alignment: .leading, spacing: 8) {
                field(suggestions, highlighted: highlighted)
                SuggestionRows(
                    kind: kind, rows: suggestions.rows, highlighted: highlighted,
                    hover: { hover($0, in: suggestions) }, pick: { pick(suggestions.rows[$0]) })
            }
        }
    }

    private func field(_ suggestions: NameSuggestions, highlighted: Int?) -> some View {
        HStack(spacing: 4) {
            Image(systemName: "plus")
                .font(.system(size: 9, weight: .bold))
                .foregroundStyle(Palette.tertiary)
            NameTextField(
                text: $text,
                placeholder: kind.prompt,
                focusOnAppear: focusOnAppear,
                focusRequests: focusRequests,
                onFocus: { focused = $0 },
                onMove: { move(by: $0, in: suggestions, from: highlighted) },
                onSubmit: { if let highlighted { pick(suggestions.rows[highlighted]) } },
                onEscape: {
                    text = ""
                    dismiss?()
                },
                onTab: tab.map { tab in
                    {
                        text = ""
                        tab()
                    }
                })
        }
        .padding(.horizontal, 9)
        .frame(height: 22)
        // The + and the padding take a click too.
        .contentShape(Capsule())
        .onTapGesture { focusRequests += 1 }
        .background(Capsule().fill(focused ? Palette.field : .clear))
        .overlay {
            // Drawn over the field, so it must let clicks through to it.
            Capsule()
                .strokeBorder(
                    focused ? Palette.accent : Palette.line,
                    style: StrokeStyle(lineWidth: 1, dash: focused ? [] : [3, 2]))
                .allowsHitTesting(false)
        }
        .onChange(of: text) { moved = nil }
    }

    private func hover(_ index: Int, in suggestions: NameSuggestions) {
        moved = suggestions.rows[index]
    }

    /// ↑↓ move to the next row that can be picked, stopping at the ends.
    private func move(by offset: Int, in suggestions: NameSuggestions, from current: Int?) {
        let choosable = suggestions.rows.indices.filter { suggestions.rows[$0].isChoosable }
        guard !choosable.isEmpty else { return }
        let next: Int?
        if let current, let position = choosable.firstIndex(of: current) {
            next = choosable[min(max(position + offset, 0), choosable.count - 1)]
        } else {
            next = offset > 0 ? choosable.first : choosable.last
        }
        moved = next.map { suggestions.rows[$0] }
    }

    private func pick(_ row: NameSuggestions.Row) {
        guard row.isChoosable else { return }
        choose(row.name)
        text = ""
        moved = nil
    }
}

/// The names a typed name could mean, as menu rows. Past eight rows they
/// scroll, keeping the highlighted one in view.
private struct SuggestionRows: View {
    let kind: GroupKind
    let rows: [NameSuggestions.Row]
    let highlighted: Int?
    let hover: (Int) -> Void
    let pick: (Int) -> Void

    private static let rowHeight: CGFloat = 26
    private static let dividerHeight: CGFloat = 9

    private var height: CGFloat {
        let divider = rows.count > 1 && rows.last.map { if case .create = $0 { true } else { false } } == true
        let content = CGFloat(rows.count) * Self.rowHeight + (divider ? Self.dividerHeight : 0)
        return min(content, 8 * Self.rowHeight)
    }

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(rows.enumerated()), id: \.element) { index, row in
                        if case .create = row, index > 0 {
                            Divider().padding(.vertical, 4).padding(.horizontal, 8)
                        }
                        Button {
                            pick(index)
                        } label: {
                            label(row, highlighted: index == highlighted)
                        }
                        .buttonStyle(.plain)
                        .disabled(!row.isChoosable)
                        .onHover { if $0 && row.isChoosable { hover(index) } }
                        .id(row)
                    }
                }
            }
            .frame(height: height)
            .scrollBounceBehavior(.basedOnSize)
            .onChange(of: highlighted) {
                if let highlighted { proxy.scrollTo(rows[highlighted]) }
            }
        }
        .font(.system(size: 13))
    }

    private func label(_ row: NameSuggestions.Row, highlighted: Bool) -> some View {
        let primary = highlighted ? Palette.onAccent : Palette.text
        let secondary = highlighted ? Palette.onAccentSecondary : Palette.tertiary
        return HStack(spacing: 8) {
            switch row {
            case .existing(let name, let count):
                Image(systemName: kind.icon).foregroundStyle(highlighted ? Palette.onAccent : Palette.accent)
                Text(name).foregroundStyle(primary).lineLimit(1)
                Spacer(minLength: 8)
                Text(count == 0 ? "Empty" : count == 1 ? "1 word" : "\(count) words").foregroundStyle(secondary)
            case .added(let name):
                Image(systemName: "checkmark").foregroundStyle(Palette.tertiary)
                Text(name).foregroundStyle(Palette.tertiary).lineLimit(1)
                Spacer(minLength: 8)
                Text("Added").foregroundStyle(Palette.tertiary)
            case .create(let name):
                Image(systemName: "plus.circle").foregroundStyle(highlighted ? Palette.onAccent : Palette.accent)
                Text("Create \(kind.noun) “\(name)”").foregroundStyle(primary).lineLimit(1)
                Spacer(minLength: 0)
            }
        }
        .padding(.horizontal, 8)
        .frame(height: Self.rowHeight)
        .background(
            RoundedRectangle(cornerRadius: 6, style: .continuous)
                .fill(highlighted ? Palette.accentFill : .clear))
        .contentShape(Rectangle())
    }
}

/// The name field itself. An AppKit field, as `SearchField` is: arrows,
/// Return, and Esc arrive only after the input method passes on them, so
/// choosing an IME candidate never picks a name. It also says when it gains
/// and loses the keyboard, which shows and hides the menu.
private struct NameTextField: NSViewRepresentable {
    @Binding var text: String
    let placeholder: String
    var focusOnAppear = false
    var focusRequests = 0
    var onFocus: (Bool) -> Void
    var onMove: (Int) -> Void
    var onSubmit: () -> Void
    var onEscape: () -> Void
    var onTab: (() -> Void)?

    func makeNSView(context: Context) -> FocusReportingField {
        let field = FocusReportingField()
        field.isBordered = false
        field.drawsBackground = false
        field.focusRingType = .none
        field.font = .systemFont(ofSize: 12)
        field.textColor = .labelColor
        field.placeholderString = placeholder
        field.setAccessibilityLabel(placeholder)
        field.cell?.usesSingleLineMode = true
        field.cell?.isScrollable = true
        field.delegate = context.coordinator
        field.onFocus = { [weak coordinator = context.coordinator] in coordinator?.parent.onFocus($0) }
        field.focusOnAppear = focusOnAppear
        return field
    }

    func updateNSView(_ field: FocusReportingField, context: Context) {
        context.coordinator.parent = self
        let composing = (field.currentEditor() as? NSTextView)?.hasMarkedText() ?? false
        if field.stringValue != text && !composing {
            field.stringValue = text
        }
        if context.coordinator.focusRequests != focusRequests {
            context.coordinator.focusRequests = focusRequests
            // After this update: taking the keyboard reports focus, which
            // changes state.
            DispatchQueue.main.async {
                if field.currentEditor() == nil { field.window?.makeFirstResponder(field) }
            }
        }
    }

    func makeCoordinator() -> Coordinator {
        let coordinator = Coordinator(parent: self)
        coordinator.focusRequests = focusRequests
        return coordinator
    }

    @MainActor
    final class Coordinator: NSObject, NSTextFieldDelegate {
        var parent: NameTextField
        var focusRequests = 0

        init(parent: NameTextField) {
            self.parent = parent
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
                control.window?.makeFirstResponder(nil)
            case #selector(NSResponder.insertTab(_:)) where parent.onTab != nil:
                parent.onTab?()
            default:
                return false
            }
            return true
        }
    }
}

/// An `NSTextField` that reports focus. Its field editor takes the keyboard
/// as soon as the field does, and gives it up through `textDidEndEditing`.
private final class FocusReportingField: NSTextField {
    var onFocus: (@MainActor (Bool) -> Void)?
    var focusOnAppear = false

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if focusOnAppear, let window {
            focusOnAppear = false
            DispatchQueue.main.async { [weak self] in
                guard let self, self.window === window else { return }
                window.makeFirstResponder(self)
            }
        }
    }

    override func becomeFirstResponder() -> Bool {
        let became = super.becomeFirstResponder()
        if became { onFocus?(true) }
        return became
    }

    override func textDidEndEditing(_ notification: Notification) {
        super.textDidEndEditing(notification)
        onFocus?(false)
    }
}
