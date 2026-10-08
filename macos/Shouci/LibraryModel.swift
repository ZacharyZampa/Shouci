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
            case newSmartCollection(LibraryFilter)
            case renameSmartCollection(String)
        }

        let id = UUID()
        let purpose: Purpose
        var text: String
    }

    /// A tag or collection to fold into another, waiting for a yes.
    struct Merging: Identifiable {
        let kind: GroupKind
        let from: String
        let into: String

        var id: String { "\(kind.noun)\n\(from)\n\(into)" }
    }

    /// Filing words into collections, one after another (`Filing.swift`).
    struct Filing: Identifiable {
        let id = UUID()
        /// The words to file, in list order.
        let queue: [Int64]
        /// Several words were selected: they are filed together, at once.
        let together: Bool
        var position = 0
        var filed = 0

        /// The word being filed now; nil when filing together, or when every
        /// word has had its turn.
        var current: Int64? { together || position >= queue.count ? nil : queue[position] }
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
    private(set) var smartCollections: [SmartCollectionView] = []
    private(set) var dictionaries: [DictionaryView] = []
    private(set) var connectors: [ConnectorView] = []
    private(set) var isLoaded = false
    /// The current view's words, sorted and grouped, and how many words the
    /// fixed views hold. Worked out when the library, the view, or the sort
    /// changes, not on every redraw: one click redraws several times, and
    /// for 15,000 words each grouping takes milliseconds.
    private(set) var listGroups: [Group] = []
    private(set) var counts: [Scope: Int] = [:]
    /// Words the list shows: the view's, narrowed by the filter.
    private(set) var shownCount = 0

    var scope: Scope = .all {
        didSet {
            if scope != oldValue {
                selection = []
                adoptFilter()
                refreshList()
            }
        }
    }
    /// The filter panel's conditions (`SmartCollections.swift`): on top of
    /// the view for the Library views, collections, and tags, and the whole
    /// of a smart collection when one is shown. Change it with `setFilter`,
    /// which asks the core for the words it matches.
    var filter = LibraryFilter()
    var showingFilter = false
    var sort: Sort = .added {
        didSet { if sort != oldValue { refreshList() } }
    }
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
    /// What went wrong, shown in the editor or filing sheet when one is
    /// open, otherwise in an alert.
    var problem: String? {
        // The window's alert is read out by itself; a notice in the editor
        // or filing sheet isn't.
        didSet { if let problem, problem != oldValue, editor != nil || filing != nil { VoiceOver.say(problem) } }
    }
    var editor: WordDraft?
    var naming: Naming?
    var merging: Merging?
    var filing: Filing?
    /// Collections words were filed into, most recent first: ↵ files the
    /// next word where the last one went.
    var recentCollections: [String] = []
    var transfer: Transfer?
    private(set) var focusSearchRequests = 0

    /// A sheet is open over the window.
    var showsSheet: Bool { editor != nil || filing != nil || transfer != nil }

    /// Every word by id. Not observed itself: SwiftUI's observation compares
    /// a new value with the old before saying it changed, which for 15,000
    /// words takes milliseconds on every reload. Read it through `words`.
    @ObservationIgnored private var byID: [Int64: ItemView] = [:]

    /// `byID`, observing `items` instead, which a reload replaces with it:
    /// the detail shows a word again when a change to it reloads the
    /// library.
    private var words: [Int64: ItemView] {
        _ = items
        return byID
    }
    /// `listGroups`' rows in order, for selection.
    @ObservationIgnored private var listOrder: [Pick] = []
    /// The same rows as a set, to find one without walking the list.
    @ObservationIgnored private var listed: Set<Pick> = []
    /// When the list was last worked out: headings such as Today go stale
    /// at midnight.
    @ObservationIgnored private var listedAt = Date.distantPast
    @ObservationIgnored private var dates: [Int64: ItemDates] = [:]
    /// The words `filter` matches, as the core worked them out; nil when no
    /// filter narrows the view.
    @ObservationIgnored var filtered: Set<Int64>?
    @ObservationIgnored var filtering: Task<Void, Never>?
    @ObservationIgnored private var version: Int64?
    @ObservationIgnored private var searching: Task<Void, Never>?
    @ObservationIgnored private var watching: Task<Void, Never>?
    /// The latest change queued for the library; the next one waits for it.
    @ObservationIgnored private var lastChange: Task<Void, Never>?
    /// Notes being typed in the detail and not saved yet. The editor starts
    /// from them, and quitting saves them.
    @ObservationIgnored var typedNotes: (id: Int64, text: String)?
    /// The library window's undo manager, where ⌘Z takes back changes
    /// (`LibraryUndo.swift`). The window's own, set by its controller:
    /// SwiftUI supplies it and never asks the window's delegate for one.
    @ObservationIgnored var undoManager: () -> UndoManager? = { nil }

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
            let query = filterQuery
            let previousDates = dates
            let (version, loaded, groups, smart, matched, dictionaries, connectors) = try await app.call { core in
                let version = try core.dataVersion()
                let items = try core.listItems(filter: LibraryFilter(view: .all))
                    + core.listItems(filter: LibraryFilter(view: .trash))
                // Worked out here, off the main thread, and only for the
                // words that changed: for 15,000 words, all the dates take
                // longer than a frame.
                let byID = Dictionary(items.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
                return (
                    version,
                    (items, byID, ItemDates.of(items, reusing: previousDates)),
                    (try core.tags(), try core.collections()),
                    try core.smartCollections(),
                    try query.map { try core.matchingIds(filter: $0) },
                    try core.dictionaries(),
                    core.connectors()
                )
            }
            let (tags, collections) = groups
            (items, byID, dates) = loaded
            self.tags = tags
            self.collections = collections
            let previous = smartCollections
            smartCollections = smart
            self.dictionaries = dictionaries
            self.connectors = connectors
            self.version = version
            let shown = listOrder
            switch scope {
            case .collection(let name) where !collections.contains(where: { $0.name == name }): scope = .all
            case .tag(let name) where !tags.contains(where: { $0.name == name }): scope = .all
            case .smart(let name) where !smart.contains(where: { $0.name == name }): scope = .all
            default: break
            }
            refilter(after: previous, matched: query == filterQuery ? matched : nil)
            refreshList()
            keepSelectionInView(shown: shown)
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
                if !Calendar.current.isDate(self.listedAt, inSameDayAs: .now) { self.refreshList() }
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

    func item(_ id: Int64) -> ItemView? { words[id] }

    /// Words in a view of the Library section, or in the current view.
    func count(of scope: Scope) -> Int { counts[scope] ?? 0 }

    /// After a reload, drops selected words that left the view: trashed,
    /// archived, filed out of Not in a Collection. When every selected word
    /// left, the word that took the first one's place is selected instead,
    /// as Mail does, so a run of ⌫ or filing goes on down the list. `shown`
    /// is the list before the reload. Search results keep what they show
    /// until the search runs again.
    private func keepSelectionInView(shown: [Pick]) {
        guard !isSearching else {
            selection = selection.filter { pick in
                if case .item(let id) = pick { words[id] != nil } else { true }
            }
            return
        }
        let kept = selection.filter(listed.contains)
        guard kept.isEmpty, let first = shown.firstIndex(where: selection.contains) else {
            selection = kept
            return
        }
        let next = shown[first...].first { listed.contains($0) && !selection.contains($0) } ?? listOrder.last
        selection = next.map { [$0] } ?? []
    }

    /// Works out what the list shows for the current view, filter, and
    /// sort.
    func refreshList() {
        let now = Date.now
        var counts = LibraryList.counts(items)
        var words = LibraryList.items(items, in: scope, dates: dates, now: now)
        counts[scope] = words.count
        if let filtered { words = words.filter { filtered.contains($0.id) } }
        shownCount = words.count
        listGroups = LibraryList.groups(words, scope: scope, sort: sort, dates: dates, now: now)
        listOrder = listGroups.flatMap { $0.rows.map(\.id) }
        listed = Set(listOrder)
        self.counts = counts
        listedAt = now
    }

    var isSearching: Bool { !query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }

    var title: String {
        if isSearching { return "Search" }
        switch scope {
        case .all: return "All Vocabulary"
        case .needsReview: return "Needs Review"
        case .noCollection: return "Not in a Collection"
        case .recent: return "Recently Added"
        case .archived: return "Archived"
        case .trash: return "Trash"
        case .collection(let name), .tag(let name), .smart(let name): return name
        }
    }

    var subtitle: String {
        let selected = selectedItems.count
        if selected > 1 { return "\(selected) selected" }
        if isSearching {
            let count = hits.count + additions.count
            return count == 1 ? "1 result" : "\(count) results"
        }
        return shownCount == 1 ? "1 word" : "\(shownCount) words"
    }

    /// The current view as something an export can ask for: the words it
    /// shows, filter and all (`savableFilter`). All Vocabulary unfiltered is
    /// "Everything", the export's own choice; the Trash can't be exported,
    /// nor Recently Added unfiltered.
    var exportFilter: ExportFilter? {
        switch scope {
        case .smart(let name): ExportFilter(filter: filter, label: name)
        case .trash: nil
        case .all where !isFiltering, .recent where !isFiltering: nil
        default: ExportFilter(filter: savableFilter, label: isFiltering ? "\(title), Filtered" : title)
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
        return listGroups
    }

    /// Rows in list order, for moving the selection from the search field.
    var order: [Pick] { isSearching ? groups.flatMap { $0.rows.map(\.id) } : listOrder }

    var selectedItems: [ItemView] {
        // None or one: read on every redraw, so not by walking the list.
        if selection.count < 2 {
            guard case .item(let id)? = selection.first, let item = words[id],
                isSearching ? order.contains(.item(id)) : listed.contains(.item(id))
            else { return [] }
            return [item]
        }
        return order.compactMap { pick in
            guard selection.contains(pick), case .item(let id) = pick else { return nil }
            return words[id]
        }
    }

    var selectedItem: ItemView? {
        guard selection.count == 1, case .item(let id)? = selection.first else { return nil }
        return words[id] ?? hits.first { $0.id == id }
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
                VoiceOver.sayWhenSettled(spokenResults)
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
        VoiceOver.say(spoken(order[next]), interrupt: true)
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
        if let item = words[id] {
            scope = item.lifecycle == .trashed ? .trash : item.lifecycle == .archived ? .archived : .all
        }
        selection = [.item(id)]
    }

    // MARK: Changes

    /// Runs `work` after the changes asked for before it, so changes to a
    /// word land in the order they were made, then reloads. Failing says
    /// why, and the task's value is then `failed`.
    @discardableResult
    func queued<Value: Sendable>(
        failed: Value, _ work: @escaping @MainActor () async throws -> Value
    ) -> Task<Value, Never> {
        let previous = lastChange
        let task = Task { () -> Value in
            await previous?.value
            let value: Value
            do {
                value = try await work()
            } catch {
                problem = describe(error)
                value = failed
            }
            await reload()
            return value
        }
        lastChange = Task { _ = await task.value }
        return task
    }

    /// Runs a change in turn (`queued`). Changes ⌘Z can take back go through
    /// `record` instead.
    func enqueue(_ work: @escaping @Sendable (Core) throws -> Void) {
        queued(failed: ()) { [app] in try await app.call(work) }
    }

    func perform(_ action: BulkAction, on ids: [Int64]) {
        guard !ids.isEmpty else { return }
        // Deleting for good can't be taken back.
        if case .purge = action {
            enqueue { _ = try $0.bulk(ids: ids, action: action) }
            return
        }
        record(action.title, ids: ids) { _ = try $0.bulk(ids: ids, action: action) }
    }

    /// Changes one word where the detail shows it: its notes, a tag, a
    /// collection. The fields and the groups change together or not at all.
    func update(_ id: Int64, patch: ItemPatch = ItemPatch(), actions: [BulkAction] = []) {
        let fieldsKept = patch.simplified == nil && patch.traditional == nil && patch.pinyin == nil
            && patch.definition == nil && patch.verification == nil
        let name =
            if fieldsKept && patch.notes == nil && actions.count == 1 { actions[0].title }
            else if fieldsKept && actions.isEmpty { "Edit Notes" }
            else { "Edit" }
        record(name, ids: [id]) { _ = try $0.editItem(id: id, patch: patch, actions: actions) }
    }

    /// Saves notes still being typed, then waits for the changes asked for:
    /// quitting loses none of them.
    func finishChanges() async {
        if let typed = typedNotes, let item = words[typed.id] {
            let notes = typed.text.trimmingCharacters(in: .whitespacesAndNewlines)
            if notes != item.notes { update(typed.id, patch: ItemPatch(notes: notes)) }
        }
        typedNotes = nil
        await lastChange?.value
    }

    func emptyTrash() {
        // In turn, so words trashed just before are emptied too.
        enqueue { _ = try $0.emptyTrash() }
    }

    func add(_ candidate: CandidateView, needsReview: Bool = false, collection: String? = nil) {
        let save: @Sendable (Core) throws -> SaveResult = { core in
            let saved = try core.saveCandidate(candidate: candidate)
            let id = saved.item.id
            if needsReview { _ = try core.bulk(ids: [id], action: .setVerification(.needsReview)) }
            if let collection { _ = try core.bulk(ids: [id], action: .addToCollection(collection)) }
            return saved
        }
        if let saved = candidate.saved {
            // Saved already, perhaps in the Trash: undoing puts it back as it was.
            let recorded = record("Add Word", ids: [saved.id]) { _ = try save($0) }
            Task { if await recorded.value { selection = [.item(saved.id)] } }
        } else {
            let added = recordAdd("Add Word", save)
            Task { if let id = await added.value { selection = [.item(id)] } }
        }
    }

    func useDefinition(of item: ItemView, from dictionary: String) {
        let id = item.id
        record("Replace Definition", ids: [id]) { _ = try $0.useDefinition(itemId: id, dictionary: dictionary) }
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
            record("New Collection", ids: ids) { core in
                try core.createCollection(name: name)
                if !ids.isEmpty { _ = try core.bulk(ids: ids, action: .addToCollection(name)) }
            }
        case .newCollectionFor(let candidate):
            let created = record("New Collection", ids: []) { try $0.createCollection(name: name) }
            Task { if await created.value { add(candidate, collection: name) } }
        case .renameCollection(let old):
            if let taken = Self.taken(name, in: collections, besides: old) {
                merging = Merging(kind: .collection, from: old, into: taken)
                return
            }
            // Asked before the change: reloading leaves a view whose name is
            // gone.
            let showing = scope == .collection(old)
            let renamed = record("Rename Collection", reading: members(.collection, old)) {
                try $0.renameCollection(from: old, to: name)
            }
            Task { if await renamed.value, showing { scope = .collection(name) } }
        case .renameTag(let old):
            if let taken = Self.taken(name, in: tags, besides: old) {
                merging = Merging(kind: .tag, from: old, into: taken)
                return
            }
            let showing = scope == .tag(old)
            let renamed = record("Rename Tag", reading: members(.tag, old)) { try $0.renameTag(from: old, to: name) }
            Task { if await renamed.value, showing { scope = .tag(name) } }
        case .newSmartCollection(let filter):
            createSmartCollection(name, filter: filter)
        case .renameSmartCollection(let old):
            renameSmartCollection(old, to: name)
        }
    }

    /// Another group already called `name`, as the library compares names
    /// (ignoring case): renaming onto it is a merge.
    private static func taken(_ name: String, in groups: [GroupView], besides old: String) -> String? {
        let wanted = name.lowercased()
        guard wanted != old.lowercased() else { return nil }
        return groups.first { $0.name.lowercased() == wanted }?.name
    }

    func merge(_ merging: Merging) {
        let (from, into) = (merging.from, merging.into)
        switch merging.kind {
        case .collection:
            let showing = scope == .collection(from)
            let merged = record("Merge Collections", reading: members(.collection, from)) {
                try $0.mergeCollections(from: from, into: into)
            }
            Task { if await merged.value, showing { scope = .collection(into) } }
        case .tag:
            let showing = scope == .tag(from)
            let merged = record("Merge Tags", reading: members(.tag, from)) { try $0.mergeTags(from: from, into: into) }
            Task { if await merged.value, showing { scope = .tag(into) } }
        }
    }

    func deleteCollection(_ name: String) {
        record("Delete Collection", reading: members(.collection, name)) { try $0.deleteCollection(name: name) }
    }

    func deleteTag(_ name: String) {
        record("Delete Tag", reading: members(.tag, name)) { try $0.deleteTag(name: name) }
    }

    func makeCollection(fromTag tag: String) {
        let made = record("Make a Collection", reading: members(.tag, tag)) { _ = try $0.collectionFromTag(tag: tag) }
        Task { if await made.value { scope = .collection(tag) } }
    }

    /// Every saved word with the tag, or in the collection, the Trash too,
    /// read as the change runs: `items` is as of the last reload, before
    /// changes still waiting their turn, or made by `shouci` since.
    private func members(_ kind: GroupKind, _ name: String) -> @Sendable (Core) throws -> [Int64] {
        let filter = kind == .tag ? LibraryFilter(view: .all, tags: [name]) : LibraryFilter(view: .all, collection: name)
        return { core in
            try [ShouciCore.LibraryView.all, .trash].flatMap { view in
                var filter = filter
                filter.view = view
                return try core.matchingIds(filter: filter)
            }
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
        problem = nil
        let draft = draft.trimmed
        guard !draft.simplified.isEmpty else {
            problem = "A word needs its characters."
            return false
        }
        // Queued after the changes made in the detail, so what the editor
        // saves wins.
        let id: Int64
        if let original = draft.original {
            let patch = draft.patch(from: original) ?? ItemPatch()
            let changes = draft.groupChanges(from: original)
            id = original.id
            // One call, so the fields and the groups change together or not
            // at all.
            guard await record("Edit", ids: [id], { core in
                _ = try core.editItem(id: id, patch: patch, actions: changes)
            }).value else { return false }
        } else {
            let word = ManualWord(
                simplified: draft.simplified,
                traditional: draft.traditional.isEmpty ? nil : draft.traditional,
                pinyin: draft.pinyin.isEmpty ? nil : draft.pinyin,
                definition: draft.definition.isEmpty ? nil : draft.definition,
                notes: draft.notes.isEmpty ? nil : draft.notes,
                tags: draft.tags, collections: draft.collections)
            let needsReview = draft.needsReview
            let known: @Sendable (Core) throws -> [Int64] = { try $0.manualMatch(word: word).map { [$0] } ?? [] }
            guard let added = await recordAdd("Add Word", known: known, { core in
                let saved = try core.addManual(word: word)
                if needsReview && saved.item.verification == .confirmed {
                    _ = try core.bulk(ids: [saved.item.id], action: .setVerification(.needsReview))
                }
                return saved
            }).value else { return false }
            id = added
        }
        if !isSearching { selection = [.item(id)] }
        return true
    }
}
