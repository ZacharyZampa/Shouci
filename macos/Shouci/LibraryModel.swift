import Foundation
import Observation
import ShouciCore

/// The library window's state (mockups 01–04). The whole library is loaded
/// at once, so scopes, counts, and sorting are instant; search and every
/// change go to the core. What the list shows for a scope is worked out in
/// `LibraryList.swift`; the editor's draft is `WordDraft.swift`.
@MainActor
@Observable
final class LibraryModel {
    /// A name being typed: a new collection, or a rename.
    struct Naming: Identifiable {
        enum Purpose {
            case newCollection(then: [Int64])
            case newCollectionFor(CandidateView)
            case renameCollection(String)
            case renameTag(String)
        }

        let id = UUID()
        let purpose: Purpose
        var text: String
    }

    /// The import or export sheet.
    enum Transfer: String, Identifiable {
        case importing
        case exporting

        var id: Self { self }
    }

    /// The library's current view, as an export filter.
    struct ExportFilter {
        let filter: LibraryFilter
        let label: String
    }

    let app: AppModel
    let preferences: Preferences

    private(set) var items: [ItemView] = []
    private(set) var tags: [GroupView] = []
    private(set) var collections: [GroupView] = []
    private(set) var dictionaries: [DictionaryView] = []
    private(set) var connectors: [ConnectorView] = []
    private(set) var isLoaded = false

    var scope: Scope = .all {
        didSet { if scope != oldValue { selection = [] } }
    }
    var sort: Sort = .added
    var query = "" {
        didSet { if query != oldValue { search() } }
    }
    var kind: QueryKind? {
        didSet { if kind != oldValue { search() } }
    }
    private(set) var hits: [ItemView] = []
    private(set) var candidates: [CandidateView] = []
    private(set) var searchedKind: QueryKind?
    var selection: Set<Pick> = []
    var problem: String?
    var editor: WordDraft?
    var naming: Naming?
    var transfer: Transfer?
    private(set) var focusSearchRequests = 0

    @ObservationIgnored private var byID: [Int64: ItemView] = [:]
    @ObservationIgnored private var dates: [Int64: ItemDates] = [:]
    @ObservationIgnored private var version: Int64?
    @ObservationIgnored private var searching: Task<Void, Never>?
    @ObservationIgnored private var watching: Task<Void, Never>?
    /// The latest change queued for the library; the next one waits for it.
    @ObservationIgnored private var lastChange: Task<Void, Never>?
    /// Notes being typed in the detail and not saved yet. The editor starts
    /// from them, and quitting saves them.
    @ObservationIgnored var typedNotes: (id: Int64, text: String)?

    init(app: AppModel, preferences: Preferences) {
        self.app = app
        self.preferences = preferences
    }

    // MARK: Loading

    func reload() async {
        guard app.core != nil else { return }
        do {
            // The version first: a change landing while the rest loads then
            // shows up as a newer version, and the watcher loads again.
            let (version, all, trash, tags, collections, dictionaries, connectors) = try await app.call { core in
                (
                    try core.dataVersion(),
                    try core.listItems(filter: LibraryFilter(view: .all)),
                    try core.listItems(filter: LibraryFilter(view: .trash)),
                    try core.tags(), try core.collections(), try core.dictionaries(),
                    core.connectors()
                )
            }
            items = all + trash
            byID = Dictionary(items.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
            dates = byID.mapValues(ItemDates.init)
            self.tags = tags
            self.collections = collections
            self.dictionaries = dictionaries
            self.connectors = connectors
            self.version = version
            switch scope {
            case .collection(let name) where !collections.contains(where: { $0.name == name }): scope = .all
            case .tag(let name) where !tags.contains(where: { $0.name == name }): scope = .all
            default: break
            }
            selection = selection.filter { pick in
                if case .item(let id) = pick { byID[id] != nil } else { true }
            }
            isLoaded = true
            if isSearching { search() }
        } catch {
            problem = describe(error)
        }
    }

    /// Reloads whenever the library changes: a word saved from quick search,
    /// or the CLI in another process.
    func startWatching() {
        watching?.cancel()
        watching = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(1.5))
                guard let self else { return }
                guard self.app.core != nil else { continue }
                let now = try? await self.app.call { try $0.dataVersion() }
                if let now, now != self.version { await self.reload() }
            }
        }
    }

    func stopWatching() {
        watching?.cancel()
        watching = nil
    }

    // MARK: What the window shows

    func item(_ id: Int64) -> ItemView? { byID[id] }

    func items(in scope: Scope) -> [ItemView] {
        LibraryList.items(items, in: scope, dates: dates, now: .now)
    }

    var isSearching: Bool { !query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }

    var title: String {
        if isSearching { return "Search" }
        switch scope {
        case .all: return "All Vocabulary"
        case .needsReview: return "Needs Review"
        case .recent: return "Recently Added"
        case .archived: return "Archived"
        case .trash: return "Trash"
        case .collection(let name), .tag(let name): return name
        }
    }

    var subtitle: String {
        let selected = selectedItems.count
        if selected > 1 { return "\(selected) selected" }
        if isSearching {
            let count = hits.count + additions.count
            return count == 1 ? "1 result" : "\(count) results"
        }
        let count = items(in: scope).count
        return count == 1 ? "1 word" : "\(count) words"
    }

    /// The current view as something an export can ask for. All Vocabulary
    /// is "Everything"; Recently Added and the Trash can't be exported.
    var exportFilter: ExportFilter? {
        switch scope {
        case .needsReview: ExportFilter(filter: LibraryFilter(view: .active, verification: .needsReview), label: "Needs Review")
        case .archived: ExportFilter(filter: LibraryFilter(view: .archived), label: "Archived")
        case .collection(let name): ExportFilter(filter: LibraryFilter(view: .all, collection: name), label: name)
        case .tag(let name): ExportFilter(filter: LibraryFilter(view: .all, tags: [name]), label: name)
        case .all, .recent, .trash: nil
        }
    }

    /// Dictionary results not already in the vocabulary.
    var additions: [CandidateView] { candidates.filter { !$0.isInVocabulary } }

    var groups: [Group] {
        if isSearching {
            var groups: [Group] = []
            if !hits.isEmpty {
                groups.append(Group(title: "In your vocabulary · \(hits.count)", rows: hits.map(Row.item)))
            }
            if !additions.isEmpty {
                groups.append(Group(title: "Add from dictionary · \(additions.count)", rows: additions.map(Row.candidate)))
            }
            return groups
        }
        return LibraryList.groups(items(in: scope), scope: scope, sort: sort, dates: dates, now: .now)
    }

    /// Rows in list order, for moving the selection from the search field.
    private var order: [Pick] { groups.flatMap { $0.rows.map(\.id) } }

    var selectedItems: [ItemView] {
        order.compactMap { pick in
            guard selection.contains(pick), case .item(let id) = pick else { return nil }
            return byID[id]
        }
    }

    var selectedItem: ItemView? {
        guard selection.count == 1, case .item(let id)? = selection.first else { return nil }
        return byID[id] ?? hits.first { $0.id == id }
    }

    var selectedCandidate: CandidateView? {
        guard selection.count == 1, case .candidate(let id)? = selection.first else { return nil }
        return candidates.first { $0.id == id }
    }

    // MARK: Search

    func search() {
        searching?.cancel()
        let text = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else {
            hits = []
            candidates = []
            searchedKind = nil
            selection = selection.filter { if case .item = $0 { true } else { false } }
            return
        }
        let kind = self.kind
        let dictionary = app.isSearchable
        searching = Task {
            try? await Task.sleep(for: .milliseconds(120))
            guard !Task.isCancelled else { return }
            do {
                let (library, found) = try await app.call { core in
                    (
                        try core.searchLibrary(query: text, filter: LibraryFilter(view: .all), kind: kind, limit: 100),
                        dictionary ? try core.searchDictionary(query: text, kind: kind, limit: 20) : nil
                    )
                }
                guard !Task.isCancelled else { return }
                hits = library.items
                candidates = found?.candidates ?? []
                searchedKind = found?.kind ?? library.kind
                if selection.isEmpty || !selection.allSatisfy({ order.contains($0) }) {
                    selection = order.first.map { [$0] } ?? []
                }
            } catch {
                guard !Task.isCancelled else { return }
                problem = describe(error)
            }
        }
    }

    func focusSearch() {
        focusSearchRequests += 1
    }

    /// ↑↓ from the search field.
    func move(by offset: Int) {
        let order = self.order
        guard !order.isEmpty else { return }
        let current = selection.count == 1 ? selection.first.flatMap(order.firstIndex) : nil
        let next = min(max((current ?? -1) + offset, 0), order.count - 1)
        selection = [order[next]]
    }

    /// ↵ from the search field: add the chosen dictionary word, or edit the
    /// chosen saved one.
    func submit() {
        if let candidate = selectedCandidate {
            add(candidate)
        } else if let item = selectedItem {
            edit(item)
        } else if isSearching {
            newWord(simplified: query.trimmingCharacters(in: .whitespacesAndNewlines))
        }
    }

    func reveal(_ id: Int64) {
        query = ""
        if let item = byID[id] {
            scope = item.lifecycle == .trashed ? .trash : item.lifecycle == .archived ? .archived : .all
        }
        selection = [.item(id)]
    }

    // MARK: Changes

    /// Runs a change, then reloads. False when it failed (and says why).
    @discardableResult
    func change(_ work: @escaping @Sendable (Core) throws -> Void) async -> Bool {
        do {
            try await app.call(work)
            await reload()
            return true
        } catch {
            problem = describe(error)
            await reload()
            return false
        }
    }

    /// Runs a change after the ones asked for before it, so changes to a
    /// word land in the order they were made.
    private func enqueue(_ work: @escaping @Sendable (Core) throws -> Void) {
        let previous = lastChange
        lastChange = Task {
            await previous?.value
            await change(work)
        }
    }

    func perform(_ action: BulkAction, on ids: [Int64]) {
        guard !ids.isEmpty else { return }
        enqueue { _ = try $0.bulk(ids: ids, action: action) }
    }

    /// Changes one word where the detail shows it: its notes, a tag, a
    /// collection. The fields and the groups change together or not at all.
    func update(_ id: Int64, patch: ItemPatch = ItemPatch(), actions: [BulkAction] = []) {
        enqueue { _ = try $0.editItem(id: id, patch: patch, actions: actions) }
    }

    /// Saves notes still being typed, then waits for the changes asked for:
    /// quitting loses none of them.
    func finishChanges() async {
        if let typed = typedNotes, let item = byID[typed.id] {
            let notes = typed.text.trimmingCharacters(in: .whitespacesAndNewlines)
            if notes != item.notes { update(typed.id, patch: ItemPatch(notes: notes)) }
        }
        typedNotes = nil
        await lastChange?.value
    }

    func emptyTrash() {
        Task { await change { _ = try $0.emptyTrash() } }
    }

    func add(_ candidate: CandidateView, needsReview: Bool = false, collection: String? = nil) {
        Task {
            do {
                let id = try await app.call { core in
                    let id = try core.saveCandidate(candidate: candidate).item.id
                    if needsReview { _ = try core.bulk(ids: [id], action: .setVerification(.needsReview)) }
                    if let collection { _ = try core.bulk(ids: [id], action: .addToCollection(collection)) }
                    return id
                }
                await reload()
                selection = [.item(id)]
            } catch {
                problem = describe(error)
            }
        }
    }

    func useDefinition(of item: ItemView, from dictionary: String) {
        let id = item.id
        Task { await change { _ = try $0.useDefinition(itemId: id, dictionary: dictionary) } }
    }

    func lookup(_ item: ItemView, in dictionary: String) async throws -> [DictionaryEntryView] {
        let id = item.id
        return try await app.call { try $0.lookupIn(itemId: id, dictionary: dictionary) }
    }

    /// Dictionary entries for characters being typed in the editor.
    func entries(for characters: String) async -> [CandidateView] {
        let text = characters.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, app.isSearchable else { return [] }
        let found = try? await app.call { try $0.searchDictionary(query: text, kind: .chinese, limit: 30) }
        return (found?.candidates ?? []).filter { !$0.inferred && ($0.simplified == text || $0.traditional == text) }
    }

    // MARK: Collections and tags

    func name(_ naming: Naming) {
        let name = naming.text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty else { return }
        switch naming.purpose {
        case .newCollection(let ids):
            Task {
                await change { core in
                    try core.createCollection(name: name)
                    if !ids.isEmpty { _ = try core.bulk(ids: ids, action: .addToCollection(name)) }
                }
            }
        case .newCollectionFor(let candidate):
            Task {
                if await change({ try $0.createCollection(name: name) }) { add(candidate, collection: name) }
            }
        case .renameCollection(let old):
            Task {
                if await change({ try $0.renameCollection(from: old, to: name) }), scope == .collection(old) {
                    scope = .collection(name)
                }
            }
        case .renameTag(let old):
            Task {
                if await change({ try $0.renameTag(from: old, to: name) }), scope == .tag(old) { scope = .tag(name) }
            }
        }
    }

    func deleteCollection(_ name: String) {
        Task { await change { try $0.deleteCollection(name: name) } }
    }

    func deleteTag(_ name: String) {
        Task { await change { try $0.deleteTag(name: name) } }
    }

    func makeCollection(fromTag tag: String) {
        Task {
            if await change({ _ = try $0.collectionFromTag(tag: tag) }) { scope = .collection(tag) }
        }
    }

    // MARK: Editing

    /// Opens the editor. A note being typed in the detail goes into it; the
    /// detail saves that note as the editor opens.
    func edit(_ item: ItemView) {
        var draft = WordDraft(item: item, style: preferences.pinyinStyle)
        if let typed = typedNotes, typed.id == item.id { draft.notes = typed.text }
        editor = draft
    }

    func newWord(simplified: String = "") {
        editor = WordDraft(simplified: simplified)
    }

    /// Saves the editor; true when it can close.
    func save(_ draft: WordDraft) async -> Bool {
        let draft = draft.trimmed
        guard !draft.simplified.isEmpty else {
            problem = "A word needs its characters."
            return false
        }
        // After the changes made in the detail, so what the editor saves wins.
        await lastChange?.value
        do {
            let id: Int64
            if let original = draft.original {
                let patch = draft.patch(from: original) ?? ItemPatch()
                let changes = draft.groupChanges(from: original)
                id = original.id
                // One call, so the fields and the groups change together or
                // not at all.
                _ = try await app.call { core in
                    try core.editItem(id: id, patch: patch, actions: changes)
                }
            } else {
                let word = ManualWord(
                    simplified: draft.simplified,
                    traditional: draft.traditional.isEmpty ? nil : draft.traditional,
                    pinyin: draft.pinyin.isEmpty ? nil : draft.pinyin,
                    definition: draft.definition.isEmpty ? nil : draft.definition,
                    notes: draft.notes.isEmpty ? nil : draft.notes,
                    tags: draft.tags, collections: draft.collections)
                let needsReview = draft.needsReview
                id = try await app.call { core in
                    let item = try core.addManual(word: word).item
                    if needsReview && item.verification == .confirmed {
                        _ = try core.bulk(ids: [item.id], action: .setVerification(.needsReview))
                    }
                    return item.id
                }
            }
            await reload()
            if !isSearching { selection = [.item(id)] }
            return true
        } catch {
            problem = describe(error)
            await reload()
            return false
        }
    }
}
