import ShouciCore
import SwiftUI

/// The quick-search panel (mockups 06–09).
struct QuickSearchView: View {
    @Bindable var model: QuickSearch
    var onHeight: (CGFloat) -> Void

    private var app: AppModel { model.app }

    var body: some View {
        VStack(spacing: 0) {
            header
            Rectangle().fill(Palette.line).frame(height: 1)
            content
            footer
        }
        .frame(width: 440)
        .fixedSize(horizontal: false, vertical: true)
        .background(Palette.list)
        .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 18, style: .continuous).strokeBorder(Palette.line))
        .foregroundStyle(Palette.text)
        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { onHeight($0) }
        .frame(maxHeight: .infinity, alignment: .top)
        .onChange(of: app.dictionary) { model.refresh() }
        .onChange(of: app.core != nil) { model.refresh() }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Quick search")
    }

    // MARK: Header

    private var header: some View {
        HStack(spacing: 10) {
            Image(systemName: "magnifyingglass")
                .font(.system(size: 15, weight: .medium))
                .foregroundStyle(Palette.tertiary)
                .accessibilityHidden(true)
            SearchField(
                text: $model.query,
                focusRequests: model.focusRequests,
                onMove: model.move(by:),
                onSubmit: model.save,
                onEscape: model.close)
            if model.hasQuery, let kind = model.shownKind {
                KindMenu(kind: $model.kind, shown: kind)
            }
        }
        .padding(.horizontal, 16)
        .frame(height: 56)
    }

    // MARK: Content

    @ViewBuilder
    private var content: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let failure = app.openFailure {
                Message(
                    icon: "exclamationmark.triangle", tint: Palette.warn,
                    title: "Shouci can’t open your library", detail: failure)
            } else if model.hasQuery {
                results
            } else {
                ready
            }
        }
        .padding(.top, 4)
        .padding(.bottom, 8)
    }

    @ViewBuilder
    private var results: some View {
        if let problem = model.problem {
            Banner(icon: "exclamationmark.triangle", tint: Palette.warn, fill: Palette.warnSoft, title: problem)
        }
        switch app.dictionary {
        case .starting, .loading:
            DictionaryProgress(state: app.dictionary)
        case .failed(let message):
            DictionaryFailure(message: message, retry: app.loadDictionaries)
        case .ready:
            ForEach(Array(groupedRows.enumerated()), id: \.element.group.id) { position, entry in
                SectionTitle(entry.group.title, first: position == 0)
                if entry.group.rows.isEmpty {
                    Text("Nothing matches “\(model.query.trimmingCharacters(in: .whitespaces))”. Check the spelling, or type the word in Hanzi.")
                        .font(.system(size: 12.5))
                        .foregroundStyle(Palette.tertiary)
                        .padding(.horizontal, 18)
                        .padding(.vertical, 8)
                }
                ForEach(entry.rows, id: \.row.id) { item in
                    row(item.row, at: item.index)
                }
            }
        }
    }

    @ViewBuilder
    private var ready: some View {
        if let notice = app.notices.first {
            Banner(icon: "info.circle", tint: Palette.accent, fill: Palette.accentSoft, title: notice) {
                Button("OK", action: app.dismissNotice).buttonStyle(PillButtonStyle())
            }
        }
        if let saved = model.saved {
            SavedBanner(saved: saved, preferences: model.preferences, undo: { model.undo() })
        } else if let removed = model.removed {
            Banner(icon: "arrow.uturn.backward.circle", tint: Palette.secondary, fill: Palette.window, title: "Removed \(removed)")
        }
        if let problem = model.problem {
            Banner(icon: "exclamationmark.triangle", tint: Palette.warn, fill: Palette.warnSoft, title: problem)
        }
        switch app.dictionary {
        case .starting, .loading:
            DictionaryProgress(state: app.dictionary)
        case .failed(let message):
            DictionaryFailure(message: message, retry: app.loadDictionaries)
        case .ready:
            EmptyView()
        }
        if !model.recent.isEmpty {
            SectionTitle("Recently added", first: true)
            ForEach(model.recent) { item in
                Button {
                    model.openLibrary(item.id)
                } label: {
                    RecentRow(
                        item: item, preferences: model.preferences, isNew: item.id == model.saved?.result.item.id)
                }
                .buttonStyle(.plain)
                .help("Show in Shouci")
            }
        } else if model.saved == nil, app.isSearchable {
            Text("Words you save show up here.")
                .font(.system(size: 12.5))
                .foregroundStyle(Palette.tertiary)
                .padding(.horizontal, 18)
                .padding(.vertical, 12)
        }
    }

    private struct IndexedRow {
        let row: QuickSearch.Row
        let index: Int
    }

    /// Each group with its rows' positions in the flat selection order.
    private var groupedRows: [(group: QuickSearch.Group, rows: [IndexedRow])] {
        var next = 0
        return model.groups.map { group in
            let rows = group.rows.map { row in
                defer { next += 1 }
                return IndexedRow(row: row, index: next)
            }
            return (group, rows)
        }
    }

    private func row(_ row: QuickSearch.Row, at index: Int) -> some View {
        let selected = model.selection == index
        return Button {
            model.select(index)
            model.save()
        } label: {
            switch row {
            case .candidate(let candidate):
                WordRow(
                    hanzi: model.preferences.headword(simplified: candidate.simplified, traditional: candidate.traditional),
                    pinyin: model.preferences.pinyin(raw: candidate.pinyin, display: candidate.pinyinDisplay),
                    gloss: candidate.definitionDisplay, selected: selected,
                    trailing: trailing(for: candidate, selected: selected))
            case .saveForReview(let text):
                ReviewRow(text: text, selected: selected)
            }
        }
        .buttonStyle(.plain)
        .onHover { if $0 { model.select(index) } }
    }

    private func trailing(for candidate: CandidateView, selected: Bool) -> WordRow.Trailing {
        if candidate.isInVocabulary {
            return candidate.saved?.lifecycle == .archived ? .note("Archived") : .saved
        }
        if candidate.basis == .characterFallback { return .note("Same characters") }
        if candidate.inferred { return selected ? .save : .none }
        if candidate.saved?.lifecycle == .trashed { return selected ? .save : .note("In the trash") }
        return selected ? .save : .none
    }

    // MARK: Footer

    private var footer: some View {
        HStack(spacing: 14) {
            if model.hasQuery && app.isSearchable && !model.rows.isEmpty {
                KeyHint(key: "↑↓", action: "Choose")
                KeyHint(key: "↵", action: "Save")
                Spacer()
            } else {
                Text(footerNote).lineLimit(1)
                Spacer()
            }
            KeyHint(key: "⌘O", action: "Open Shouci")
            KeyHint(key: "esc", action: "Close")
        }
        .font(.system(size: 12))
        .foregroundStyle(Palette.tertiary)
        .padding(.horizontal, 16)
        .frame(height: 38)
        .background(Palette.window)
        .overlay(alignment: .top) { Rectangle().fill(Palette.line).frame(height: 1) }
    }

    private var footerNote: String {
        if case .ready(let updating?) = app.dictionary { return updating }
        return model.saved != nil ? "Ready for the next word" : "Type Hanzi, pinyin, or English"
    }
}

// MARK: - Pieces

private struct SectionTitle: View {
    let title: String
    let first: Bool

    init(_ title: String, first: Bool) {
        self.title = title
        self.first = first
    }

    var body: some View {
        Text(title)
            .font(.system(size: 11, weight: .semibold))
            .foregroundStyle(Palette.tertiary)
            .padding(.horizontal, 18)
            .padding(.bottom, 4)
            .frame(maxWidth: .infinity, minHeight: first ? 28 : 32, alignment: .bottomLeading)
            .accessibilityAddTraits(.isHeader)
    }
}

/// A word: characters and reading over its meaning.
struct WordRow: View {
    enum Trailing: Equatable {
        case none
        case save
        case saved
        case note(String)
        case time(String, needsReview: Bool)
    }

    let hanzi: String
    let pinyin: String
    let gloss: String
    var selected = false
    var highlighted = false
    var trailing: Trailing = .none

    var body: some View {
        HStack(spacing: 10) {
            VStack(alignment: .leading, spacing: 1) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(hanzi).font(.system(size: 18, weight: .semibold))
                    Text(pinyin).font(.system(size: 13)).foregroundStyle(secondary)
                }
                Text(gloss)
                    .font(.system(size: 12.5))
                    .foregroundStyle(secondary)
                    .lineLimit(1)
                    .truncationMode(.tail)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            trailingView
        }
        .foregroundStyle(selected ? Palette.onAccent : Palette.text)
        .padding(.horizontal, 10)
        .frame(height: 52)
        .background(
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .fill(selected ? Palette.accentFill : highlighted ? Palette.accentSoft : .clear))
        .contentShape(Rectangle())
        .padding(.horizontal, 8)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }

    private var secondary: Color { selected ? Palette.onAccentSecondary : Palette.tertiary }

    @ViewBuilder
    private var trailingView: some View {
        switch trailing {
        case .none:
            EmptyView()
        case .save:
            HStack(spacing: 6) {
                Text("Save")
                Text("↵")
                    .font(.system(size: 11))
                    .frame(minWidth: 20, minHeight: 20)
                    .background(RoundedRectangle(cornerRadius: 5).fill(Color.white.opacity(0.22)))
            }
            .font(.system(size: 12, weight: .semibold))
        case .saved:
            Label("Saved", systemImage: "checkmark")
                .labelStyle(TightLabelStyle())
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(selected ? Palette.onAccent : Palette.ok)
        case .note(let text):
            Text(text).font(.system(size: 12)).foregroundStyle(secondary)
        case .time(let text, let needsReview):
            if needsReview {
                Image(systemName: "exclamationmark.triangle")
                    .font(.system(size: 13))
                    .foregroundStyle(Palette.warn)
                    .accessibilityLabel("Needs review")
            }
            Text(text).font(.system(size: 12)).foregroundStyle(secondary)
        }
    }
}

private struct RecentRow: View {
    let item: ItemView
    let preferences: Preferences
    let isNew: Bool

    var body: some View {
        let pinyin = preferences.pinyin(raw: item.pinyin, display: item.pinyinDisplay)
        WordRow(
            hanzi: preferences.headword(simplified: item.simplified, traditional: item.traditional),
            pinyin: pinyin.isEmpty ? "—" : pinyin,
            gloss: gloss,
            highlighted: isNew,
            trailing: .time(When.short(item.createdAt), needsReview: item.verification == .needsReview))
    }

    private var gloss: String {
        if !item.definitionDisplay.isEmpty { return item.definitionDisplay }
        return item.verification == .needsReview ? "Needs review · no dictionary match" : ""
    }
}

private struct ReviewRow: View {
    let text: String
    let selected: Bool

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "exclamationmark.triangle").font(.system(size: 17))
            VStack(alignment: .leading, spacing: 2) {
                Text("Save “\(text)” as needs review").font(.system(size: 14, weight: .semibold))
                Text("Add the reading and meaning later in Shouci")
                    .font(.system(size: 12.5))
                    .foregroundStyle(selected ? Palette.onAccentSecondary : Palette.tertiary)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if selected {
                Text("↵")
                    .font(.system(size: 11))
                    .frame(minWidth: 20, minHeight: 20)
                    .background(RoundedRectangle(cornerRadius: 5).fill(Color.white.opacity(0.22)))
            }
        }
        .foregroundStyle(selected ? Palette.onAccent : Palette.text)
        .padding(10)
        .frame(minHeight: 60)
        .background(
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .fill(selected ? Palette.accentFill : .clear))
        .contentShape(Rectangle())
        .padding(.horizontal, 8)
        .accessibilityElement(children: .combine)
    }
}

private struct SavedBanner: View {
    let saved: QuickSearch.Saved
    let preferences: Preferences
    let undo: () -> Void

    var body: some View {
        let item = saved.result.item
        let pinyin = preferences.pinyin(raw: item.pinyin, display: item.pinyinDisplay)
        Banner(
            icon: "checkmark.circle", tint: Palette.ok, fill: Palette.okSoft,
            title: "\(verb) \(item.simplified)\(pinyin.isEmpty ? "" : " · \(pinyin)")",
            detail: item.definitionDisplay.isEmpty ? nil : item.definitionDisplay
        ) {
            if saved.canUndo {
                Button(action: undo) {
                    HStack(spacing: 6) {
                        Text("Undo")
                        Text("⌘Z").font(.system(size: 11)).foregroundStyle(Palette.tertiary).fontWeight(.regular)
                    }
                }
                .buttonStyle(PillButtonStyle())
                .accessibilityLabel("Undo save")
            }
        }
    }

    private var verb: String {
        switch saved.result.outcome {
        case .inserted: "Saved"
        case .alreadySaved: "Already saved:"
        case .restored: "Restored"
        case .completed: "Completed"
        }
    }
}

private struct Banner<Accessory: View>: View {
    let icon: String
    let tint: Color
    let fill: Color
    let title: String
    var detail: String?
    @ViewBuilder var accessory: () -> Accessory

    init(
        icon: String, tint: Color, fill: Color, title: String, detail: String? = nil,
        @ViewBuilder accessory: @escaping () -> Accessory = { EmptyView() }
    ) {
        self.icon = icon
        self.tint = tint
        self.fill = fill
        self.title = title
        self.detail = detail
        self.accessory = accessory
    }

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: icon).font(.system(size: 19)).foregroundStyle(tint).accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 1) {
                Text(title).font(.system(size: 14, weight: .semibold)).foregroundStyle(tint)
                if let detail {
                    Text(detail).font(.system(size: 12.5)).foregroundStyle(Palette.secondary).lineLimit(1)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            accessory()
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(fill))
        .padding(.horizontal, 8)
        .padding(.top, 4)
    }
}

private struct DictionaryProgress: View {
    let state: DictionaryState

    var body: some View {
        HStack(spacing: 10) {
            ProgressView().controlSize(.small)
            Text(text).font(.system(size: 13)).foregroundStyle(Palette.secondary)
        }
        .padding(.horizontal, 18)
        .padding(.vertical, 14)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var text: String {
        if case .loading(let text) = state { return text }
        return "Opening your library…"
    }
}

private struct DictionaryFailure: View {
    let message: String
    let retry: () -> Void

    var body: some View {
        Banner(
            icon: "exclamationmark.triangle", tint: Palette.warn, fill: Palette.warnSoft,
            title: "The dictionary isn’t available", detail: message
        ) {
            Button("Try Again", action: retry).buttonStyle(PillButtonStyle())
        }
    }
}

private struct Message: View {
    let icon: String
    let tint: Color
    let title: String
    let detail: String

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: icon).font(.system(size: 19)).foregroundStyle(tint)
            VStack(alignment: .leading, spacing: 3) {
                Text(title).font(.system(size: 14, weight: .semibold))
                Text(detail).font(.system(size: 12.5)).foregroundStyle(Palette.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(18)
    }
}

/// A small rounded button on a banner.
struct PillButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .fixedSize()
            .font(.system(size: 12, weight: .semibold))
            .foregroundStyle(Palette.text)
            .padding(.horizontal, 10)
            .frame(height: 26)
            .background(
                Capsule().fill(Palette.list).strokeBorder(Palette.line))
            .opacity(configuration.isPressed ? 0.7 : 1)
    }
}

private struct TightLabelStyle: LabelStyle {
    func makeBody(configuration: Configuration) -> some View {
        HStack(spacing: 4) {
            configuration.icon
            configuration.title
        }
    }
}
