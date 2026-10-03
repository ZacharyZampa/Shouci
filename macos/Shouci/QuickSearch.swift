import Foundation
import Observation
import ShouciCore

/// The quick-search panel's state: type, choose, save, undo.
@MainActor
@Observable
final class QuickSearch {
    enum Row: Identifiable {
        case candidate(CandidateView)
        /// No dictionary has the word: keep it as typed, to finish later.
        case saveForReview(String)

        var id: String {
            switch self {
            case .candidate(let candidate): candidate.id
            case .saveForReview(let text): "review\u{1F}\(text)"
            }
        }
    }

    struct Group: Identifiable {
        let title: String
        let rows: [Row]
        var id: String { title }
    }

    struct Saved {
        let result: SaveResult
        /// Inserting or restoring can be taken back; the rest changed nothing
        /// or only filled in a reading.
        var canUndo: Bool { result.outcome == .inserted || result.outcome == .restored }
    }

    let app: AppModel
    let preferences: Preferences
    @ObservationIgnored var close: () -> Void = {}
    /// Opens the library window, showing a word when given one.
    @ObservationIgnored var openLibrary: (Int64?) -> Void = { _ in }

    var query = "" {
        didSet { if query != oldValue { queryChanged() } }
    }
    /// How to read the query; `nil` lets the core decide.
    var kind: QueryKind? {
        didSet { if kind != oldValue { search() } }
    }

    private(set) var results: DictionaryResults?
    private(set) var groups: [Group] = []
    private(set) var selection: Int?
    private(set) var recent: [ItemView] = []
    private(set) var saved: Saved?
    /// The word an undo just removed.
    private(set) var removed: String?
    private(set) var problem: String?
    private(set) var focusRequests = 0
    private(set) var isBusy = false

    @ObservationIgnored private var searching: Task<Void, Never>?

    /// Results asked for: enough to choose from, few enough to scan.
    nonisolated static let limit: UInt32 = 8

    init(app: AppModel, preferences: Preferences) {
        self.app = app
        self.preferences = preferences
    }

    var rows: [Row] { groups.flatMap(\.rows) }
    var hasQuery: Bool { !trimmedQuery.isEmpty }
    var shownKind: QueryKind? { kind ?? results?.kind }
    private var trimmedQuery: String { query.trimmingCharacters(in: .whitespacesAndNewlines) }

    // MARK: Panel

    func opened() {
        query = ""
        kind = nil
        saved = nil
        removed = nil
        problem = nil
        focusRequests += 1
        loadRecent()
    }

    func closed() {
        searching?.cancel()
    }

    /// The library opened or the dictionary became searchable.
    func refresh() {
        loadRecent()
        search()
    }

    // MARK: Search

    private func queryChanged() {
        saved = nil
        removed = nil
        problem = nil
        search()
    }

    func search() {
        searching?.cancel()
        let text = trimmedQuery
        guard !text.isEmpty, app.isSearchable else {
            results = nil
            groups = []
            selection = nil
            return
        }
        let kind = self.kind
        searching = Task {
            try? await Task.sleep(for: .milliseconds(40))
            guard !Task.isCancelled else { return }
            do {
                let found = try await app.call { core in
                    try core.searchDictionary(query: text, kind: kind, limit: Self.limit)
                }
                guard !Task.isCancelled else { return }
                show(found)
            } catch {
                guard !Task.isCancelled else { return }
                problem = describe(error)
            }
        }
    }

    private func show(_ found: DictionaryResults) {
        problem = nil
        results = found
        let strong = found.candidates.filter { !$0.inferred }
        let partial = found.candidates.filter(\.inferred)
        var groups: [Group] = []
        if strong.isEmpty {
            let review: [Row] = found.kind == .chinese ? [.saveForReview(found.query)] : []
            groups.append(Group(title: "No dictionary match", rows: review))
            if !partial.isEmpty {
                groups.append(Group(title: "Partial matches", rows: partial.map(Row.candidate)))
            }
        } else {
            let mine = strong.filter(\.isInVocabulary)
            let others = strong.filter { !$0.isInVocabulary } + partial
            if !mine.isEmpty {
                groups.append(Group(title: "In your vocabulary", rows: mine.map(Row.candidate)))
            }
            if !others.isEmpty {
                groups.append(Group(title: "Dictionary", rows: others.map(Row.candidate)))
            }
        }
        self.groups = groups
        let rows = self.rows
        selection = rows.firstIndex { row in
            if case .candidate(let candidate) = row { !candidate.isInVocabulary } else { true }
        } ?? (rows.isEmpty ? nil : 0)
    }

    // MARK: Choosing and saving

    func move(by offset: Int) {
        let count = rows.count
        guard count > 0 else { return }
        selection = min(max((selection ?? -1) + offset, 0), count - 1)
    }

    func select(_ index: Int) {
        if rows.indices.contains(index) { selection = index }
    }

    func save() {
        guard let selection, rows.indices.contains(selection), !isBusy else { return }
        let row = rows[selection]
        isBusy = true
        Task {
            defer { isBusy = false }
            do {
                let result: SaveResult
                switch row {
                case .candidate(let candidate):
                    result = try await app.call { try $0.saveCandidate(candidate: candidate) }
                case .saveForReview(let text):
                    result = try await app.call { try $0.addManual(word: ManualWord(simplified: text)) }
                }
                didSave(result)
            } catch {
                problem = describe(error)
            }
        }
    }

    private func didSave(_ result: SaveResult) {
        searching?.cancel()
        query = ""
        saved = Saved(result: result)
        loadRecent()
        if preferences.afterSave == .close { close() }
    }

    /// Takes back the last save. False when there is nothing to take back.
    @discardableResult
    func undo() -> Bool {
        guard let saved, saved.canUndo, !isBusy else { return false }
        let item = saved.result.item
        let purge = saved.result.outcome == .inserted
        isBusy = true
        Task {
            defer { isBusy = false }
            do {
                try await app.call { core in
                    _ = try core.bulk(ids: [item.id], action: .trash)
                    if purge { _ = try core.bulk(ids: [item.id], action: .purge) }
                }
                self.saved = nil
                removed = item.simplified
                loadRecent()
            } catch {
                problem = describe(error)
            }
        }
        return true
    }

    private func loadRecent() {
        guard app.core != nil else { return }
        Task {
            let found = try? await app.call { core in
                try core.searchLibrary(query: "", filter: LibraryFilter(), kind: nil, limit: 5)
            }
            if let found { recent = found.items }
        }
    }
}

extension CandidateView {
    /// Saved and not in the trash. Saving a word from the trash brings it back.
    var isInVocabulary: Bool {
        saved.map { $0.lifecycle != .trashed } ?? false
    }
}

extension QueryKind {
    var title: String {
        switch self {
        case .chinese: "Hanzi"
        case .pinyin: "Pinyin"
        case .english: "English"
        }
    }
}
