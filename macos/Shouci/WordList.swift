import ShouciCore
import SwiftUI

/// The middle column: the words in the chosen view, grouped by date, or
/// search results split into saved words and dictionary words (mockups
/// 01–03).
struct WordList: View {
    @Bindable var model: LibraryModel

    var body: some View {
        List(selection: $model.selection) {
            ForEach(model.groups) { group in
                Section(group.title) {
                    ForEach(group.rows) { row in
                        rowView(row).tag(row.id)
                    }
                }
            }
        }
        .listStyle(.inset)
        .contextMenu(forSelectionType: LibraryModel.Pick.self) { picks in
            menu(for: picks)
        } primaryAction: { picks in
            open(picks)
        }
        .onDeleteCommand {
            let ids = model.selectedItems.filter { $0.lifecycle != .trashed }.map(\.id)
            model.perform(.trash, on: ids)
        }
        .overlay { if model.isLoaded && model.groups.isEmpty { empty } }
        .safeAreaInset(edge: .top, spacing: 0) {
            if !model.isSearching { header }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            if model.isSearching && !model.groups.isEmpty { hints }
        }
    }

    @ViewBuilder
    private func rowView(_ row: LibraryModel.Row) -> some View {
        switch row {
        case .item(let item):
            ItemRow(item: item, preferences: model.preferences)
        case .candidate(let candidate):
            CandidateRow(candidate: candidate, preferences: model.preferences) { model.add(candidate) }
        }
    }

    private var header: some View {
        HStack(spacing: 12) {
            if model.scope == .trash && !model.items(in: .trash).isEmpty {
                Button("Empty Trash…") { confirmingEmpty = true }
                    .controlSize(.small)
                    .confirmationDialog(
                        "Delete every word in the Trash for good?", isPresented: $confirmingEmpty
                    ) {
                        Button("Empty Trash", role: .destructive, action: model.emptyTrash)
                    } message: {
                        Text("This can’t be undone.")
                    }
            }
            Spacer()
            Picker("Sort", selection: $model.sort) {
                ForEach(LibraryModel.Sort.allCases) { Text($0.title).tag($0) }
            }
            .pickerStyle(.menu)
            .labelsHidden()
            .fixedSize()
            .controlSize(.small)
            .disabled(model.scope == .trash || model.scope == .archived)
            .help("Sort")
        }
        .font(.system(size: 12))
        .padding(.horizontal, 14)
        .padding(.vertical, 6)
        .background(.bar)
    }

    @State private var confirmingEmpty = false

    private var hints: some View {
        HStack(spacing: 14) {
            KeyHint(key: "↑↓", action: "Choose")
            KeyHint(key: "↵", action: model.selectedCandidate != nil ? "Add word" : "Edit")
            Spacer()
            KeyHint(key: "esc", action: "Clear")
        }
        .font(.system(size: 12))
        .foregroundStyle(Palette.tertiary)
        .padding(.horizontal, 14)
        .frame(height: 34)
        .background(.bar)
    }

    @ViewBuilder
    private var empty: some View {
        if model.isSearching {
            ContentUnavailableView {
                Label("No matches", systemImage: "magnifyingglass")
            } description: {
                Text("Nothing in your vocabulary or the dictionary matches “\(model.query)”.")
            } actions: {
                Button("Add “\(model.query)” by Hand") { model.newWord(simplified: model.query) }
            }
        } else {
            switch model.scope {
            case .all:
                ContentUnavailableView(
                    "No words yet", systemImage: "character.book.closed",
                    description: Text("Press \(model.preferences.hotKey.display) anywhere to save a word, or use Add Word."))
            case .needsReview:
                ContentUnavailableView(
                    "Nothing needs review", systemImage: "checkmark.circle",
                    description: Text("Words with no dictionary match, or an uncertain one, show up here."))
            case .recent:
                ContentUnavailableView(
                    "Nothing added this week", systemImage: "clock",
                    description: Text("Words you save in the last seven days show up here."))
            case .archived:
                ContentUnavailableView(
                    "No archived words", systemImage: "archivebox",
                    description: Text("Archive words you know to keep them out of the way."))
            case .trash:
                ContentUnavailableView("The Trash is empty", systemImage: "trash")
            case .collection, .tag:
                ContentUnavailableView("No words here yet", systemImage: "folder")
            }
        }
    }

    // MARK: Actions

    private func open(_ picks: Set<LibraryModel.Pick>) {
        guard picks.count == 1, let pick = picks.first else { return }
        switch pick {
        case .item(let id):
            if let item = model.item(id) { model.edit(item) }
        case .candidate:
            if let candidate = model.selectedCandidate { model.add(candidate) }
        }
    }

    @ViewBuilder
    private func menu(for picks: Set<LibraryModel.Pick>) -> some View {
        let items = picks.compactMap { pick -> ItemView? in
            if case .item(let id) = pick { model.item(id) } else { nil }
        }
        if items.isEmpty {
            if picks.count == 1, case .candidate(let id)? = picks.first,
                let candidate = model.candidates.first(where: { $0.id == id })
            {
                Button("Add to Vocabulary") { model.add(candidate) }
                Button("Add as Needs Review") { model.add(candidate, needsReview: true) }
            }
        } else {
            WordActions(model: model, items: items, includeEdit: true)
        }
    }
}

/// Actions on one or more saved words, for context menus and the detail's
/// More menu.
struct WordActions: View {
    let model: LibraryModel
    let items: [ItemView]
    var includeEdit = false

    var body: some View {
        let ids = items.map(\.id)
        let trashed = items.allSatisfy { $0.lifecycle == .trashed }
        if includeEdit, items.count == 1, let item = items.first, !trashed {
            Button("Edit…") { model.edit(item) }
            Divider()
        }
        if trashed {
            Button("Put Back") { model.perform(.restore, on: ids) }
            Button("Delete Immediately", role: .destructive) { model.perform(.purge, on: ids) }
        } else {
            if items.allSatisfy({ $0.verification == .needsReview }) {
                Button("Mark as Confirmed") { model.perform(.setVerification(.confirmed), on: ids) }
            } else {
                Button("Mark as Needs Review") { model.perform(.setVerification(.needsReview), on: ids) }
            }
            if items.allSatisfy({ $0.lifecycle == .archived }) {
                Button("Unarchive") { model.perform(.unarchive, on: ids) }
            } else {
                Button("Archive") { model.perform(.archive, on: ids) }
            }
            Divider()
            Button("Copy") {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(items.map(\.simplified).joined(separator: "\n"), forType: .string)
            }
            Divider()
            Button("Move to Trash") { model.perform(.trash, on: ids) }
        }
    }
}

/// A saved word in the list.
struct ItemRow: View {
    let item: ItemView
    let preferences: Preferences
    @Environment(\.backgroundProminence) private var prominence

    var body: some View {
        HStack(spacing: 10) {
            VStack(alignment: .leading, spacing: 1) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(preferences.headword(simplified: item.simplified, traditional: item.traditional))
                        .font(.system(size: 18, weight: .semibold))
                    Text(reading).font(.system(size: 13)).foregroundStyle(.secondary)
                }
                Text(gloss)
                    .font(.system(size: 12.5))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if item.verification == .needsReview {
                Image(systemName: "exclamationmark.triangle")
                    .foregroundStyle(prominence == .increased ? AnyShapeStyle(.primary) : AnyShapeStyle(Palette.warn))
                    .accessibilityLabel("Needs review")
            }
        }
        .padding(.vertical, 5)
        .accessibilityElement(children: .combine)
    }

    private var reading: String {
        let pinyin = preferences.pinyin(raw: item.pinyin, display: item.pinyinDisplay)
        return pinyin.isEmpty ? "—" : pinyin
    }

    private var gloss: String {
        if !item.definitionDisplay.isEmpty { return item.definitionDisplay }
        return item.verification == .needsReview ? "Needs review · no dictionary match" : "No definition"
    }
}

/// A dictionary word in search results, with Add.
struct CandidateRow: View {
    let candidate: CandidateView
    let preferences: Preferences
    let add: () -> Void
    @Environment(\.backgroundProminence) private var prominence

    var body: some View {
        HStack(spacing: 10) {
            VStack(alignment: .leading, spacing: 1) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(preferences.headword(simplified: candidate.simplified, traditional: candidate.traditional))
                        .font(.system(size: 18, weight: .semibold))
                    Text(preferences.pinyin(raw: candidate.pinyin, display: candidate.pinyinDisplay))
                        .font(.system(size: 13)).foregroundStyle(.secondary)
                }
                Text(candidate.definitionDisplay)
                    .font(.system(size: 12.5))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Button(action: add) {
                Label("Add", systemImage: "plus")
                    .labelStyle(TitleAndIconTight())
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(prominence == .increased ? Color.white : Palette.accent)
                    .padding(.horizontal, 10)
                    .frame(height: 24)
                    .background(Capsule().fill(prominence == .increased ? Color.white.opacity(0.22) : Palette.accentSoft))
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Add \(candidate.simplified)")
        }
        .padding(.vertical, 5)
    }
}

private struct TitleAndIconTight: LabelStyle {
    func makeBody(configuration: Configuration) -> some View {
        HStack(spacing: 4) {
            configuration.icon.font(.system(size: 10, weight: .bold))
            configuration.title
        }
    }
}
