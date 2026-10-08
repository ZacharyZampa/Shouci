// The app's logic, tested without the app: these tests compile its sources
// into the test bundle and run them against a library of their own, so
// nothing launches, and the real library, settings, and shortcut are never
// touched.

import Foundation
import ShouciCore

/// A library in a temporary folder, with no dictionary, and the library
/// window's model over it, with an undo manager of its own.
@MainActor
final class TestLibrary {
    let core: Core
    let model: LibraryModel
    let undo = UndoManager()
    private let dir: URL

    init() throws {
        dir = FileManager.default.temporaryDirectory.appendingPathComponent("shouci-tests-\(UUID().uuidString)")
        core = try Core.open(
            config: CoreConfig(
                dataDir: dir.path,
                dictionariesDir: dir.appendingPathComponent("dictionaries").path,
                fetchDictionaries: false))
        let defaults = UserDefaults(suiteName: "shouci-tests-\(UUID().uuidString)")!
        model = LibraryModel(app: AppModel(core: core), preferences: Preferences(defaults: defaults))
        let undo = undo
        model.undoManager = { undo }
    }

    deinit {
        try? FileManager.default.removeItem(at: dir)
    }

    /// Waits for the changes asked for and what they set off once done,
    /// then ends the undo group they registered in, as the end of an event
    /// does in the app.
    func settle() async {
        await model.finishChanges()
        for _ in 0..<20 { await Task.yield() }
        while undo.groupingLevel > 0 { undo.endUndoGrouping() }
    }

    /// ⌘Z.
    func undoLast() async {
        undo.undo()
        await settle()
    }

    /// ⇧⌘Z.
    func redoLast() async {
        undo.redo()
        await settle()
    }

    /// Saves a word straight into the library, as `shouci` would: the
    /// window doesn't know of it until it reloads.
    @discardableResult
    func add(_ simplified: String, tags: [String] = [], collections: [String] = []) throws -> ItemView {
        try core.addManual(word: ManualWord(simplified: simplified, tags: tags, collections: collections)).item
    }

    func word(_ id: Int64) throws -> ItemView {
        try core.item(id: id)
    }
}

/// A word as the core describes it, for the list's pure functions.
func word(
    _ id: Int64, _ simplified: String, pinyin: String = "", hsk: UInt64? = nil, frequency: UInt64? = nil,
    band: FrequencyBand = .unlisted, lifecycle: Lifecycle = .active, collections: [String] = [],
    added: String = "2026-01-01T09:00:00.000Z", trashed: String? = nil
) -> ItemView {
    ItemView(
        id: id, simplified: simplified, traditional: simplified, pinyin: pinyin, pinyinDisplay: pinyin,
        definition: "", definitionDisplay: "", notes: "", verification: .confirmed, lifecycle: lifecycle,
        source: SourceView(kind: .manual, id: nil, version: nil, importOrigin: nil),
        tags: [], collections: collections, destinations: [], frequencyRank: frequency, frequencyBand: band,
        hskRank: hsk, createdAt: added, modifiedAt: trashed ?? added, archivedAt: nil, deletedAt: trashed, rev: 1)
}
