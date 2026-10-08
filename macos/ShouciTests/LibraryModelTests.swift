import Foundation
import Observation
import ShouciCore
import Testing

/// Filing, smart collections, the filter panel, and how problems are shown.
@MainActor
struct LibraryModelTests {
    /// The words by id aren't observed themselves (comparing 15,000 of them
    /// on each reload is slow): reading one must still see a reload that
    /// changed it, or the detail would show it as it was.
    @Test func aWordShownIsSeenAgainWhenAReloadChangesIt() async throws {
        let library = try TestLibrary()
        let word = try library.add("书")
        await library.model.reload()
        let changed = Flag()
        withObservationTracking {
            _ = library.model.item(word.id)
        } onChange: {
            changed.set()
        }
        _ = try library.core.bulk(ids: [word.id], action: .setVerification(.needsReview))
        await library.model.reload()
        #expect(changed.isSet)
    }

    @Test func aWordIsCountedOnceFiledAndAFailedFilingIsNot() async throws {
        let library = try TestLibrary()
        let model = library.model
        try library.add("学校")
        try library.add("旅行")
        await model.reload()

        model.startFiling()
        let first = try #require(model.filing?.current)
        model.file(into: "Week 1")
        await library.settle()
        #expect(try library.word(first).collections == ["Week 1"])
        #expect(model.filing?.filed == 1)

        // The next word deleted for good by `shouci` in the meantime
        let next = try #require(model.filing?.current)
        _ = try library.core.bulk(ids: [next], action: .trash)
        _ = try library.core.bulk(ids: [next], action: .purge)
        model.file(into: "Week 1")
        await library.settle()
        #expect(model.filing?.filed == 1, "not counted")
        #expect(model.problem != nil, "and the filing sheet says why")
        #expect(model.showsSheet)
    }

    @Test func renamingASmartCollectionKeepsConditionsNotYetSaved() async throws {
        let library = try TestLibrary()
        let model = library.model
        _ = try library.core.createSmartCollection(name: "Drill", filter: LibraryFilter(view: .active, hskLevels: [1]))
        await model.reload()
        model.scope = .smart("Drill")
        model.setFilter(LibraryFilter(view: .active, hskLevels: [1, 2]))
        #expect(model.hasUnsavedSmartChanges)

        model.renameSmartCollection("Drill", to: "Practice")
        await library.settle()
        #expect(model.scope == .smart("Practice"))
        #expect(model.filter.hskLevels == [1, 2])
        #expect(model.hasUnsavedSmartChanges, "still to save, under the new name")
    }

    @Test func inACollectionAndInNoneTakeEachOtherOff() {
        var filter = LibraryFilter(view: .active, collection: "Week 1", anyCollections: ["Food"])
        filter.setNoCollection(true)
        #expect(filter.noCollection)
        #expect(filter.anyCollections.isEmpty && filter.collection == nil)
        filter.addAnyCollection("Food")
        #expect(filter.anyCollections == ["Food"])
        #expect(!filter.noCollection)
    }

    @Test func aNameAskedForAndKeptOutTakeEachOtherOff() {
        var filter = LibraryFilter(view: .active, tags: ["drill"], collection: "Week 1", anyTags: ["food"])
        filter.addWithoutTag("Food")
        filter.addWithoutTag("drill")
        #expect(filter.anyTags.isEmpty && filter.tags.isEmpty)
        filter.addAnyTag("food")
        #expect(filter.anyTags == ["food"])
        #expect(filter.withoutTags == ["drill"])

        filter.addAnyCollection("Food")
        filter.addWithoutCollection("Week 1")
        filter.addWithoutCollection("food")
        #expect(filter.anyCollections.isEmpty && filter.collection == nil)
        filter.addAnyCollection("Week 1")
        #expect(filter.withoutCollections == ["food"])
    }

    @Test func theEditorShowsWhyItCouldNotSave() async throws {
        let library = try TestLibrary()
        let model = library.model
        await model.reload()
        model.editor = WordDraft(simplified: " ")
        #expect(model.showsSheet, "so the window's alert waits")

        #expect(await model.save(WordDraft(simplified: " ")) == false)
        #expect(model.problem == "A word needs its characters.")
        #expect(await model.save(WordDraft(simplified: "米饭")))
        #expect(model.problem == nil, "a new try clears it")
    }
}

/// Set once, from whichever thread observation calls back on.
private final class Flag: @unchecked Sendable {
    private let lock = NSLock()
    private var value = false

    var isSet: Bool { lock.withLock { value } }

    func set() {
        lock.withLock { value = true }
    }
}
