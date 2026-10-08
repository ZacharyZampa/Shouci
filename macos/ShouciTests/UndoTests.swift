import Foundation
import ShouciCore
import Testing

/// ⌘Z in the library window: every change goes through the core's
/// snapshots, and an undo never overwrites what happened since.
@MainActor
struct UndoTests {
    @Test func aBulkChangeIsUndoneAndRedone() async throws {
        let library = try TestLibrary()
        let school = try library.add("学校")
        await library.model.reload()

        library.model.perform(.archive, on: [school.id])
        await library.settle()
        #expect(try library.word(school.id).lifecycle == .archived)
        #expect(library.undo.undoActionName == "Archive")

        await library.undoLast()
        #expect(try library.word(school.id).lifecycle == .active)
        await library.redoLast()
        #expect(try library.word(school.id).lifecycle == .archived)
    }

    @Test func anAddedWordIsTakenBackAndRedoneAsTheSameWord() async throws {
        let library = try TestLibrary()
        let model = library.model
        await model.reload()

        #expect(await model.save(WordDraft(simplified: "米饭")))
        await library.settle()
        let id = try #require(model.items.first?.id)

        await library.undoLast()
        #expect(model.items.isEmpty)
        #expect(throws: ShouciError.self) { try library.word(id) }

        await library.redoLast()
        #expect(model.items.map(\.id) == [id], "the same word, not a new one")
    }

    @Test func anAddIsNotTakenBackOnceTheWordChanged() async throws {
        let library = try TestLibrary()
        let model = library.model
        await model.reload()
        #expect(await model.save(WordDraft(simplified: "米饭")))
        await library.settle()
        let id = try #require(model.items.first?.id)

        // Written by `shouci` in the meantime
        _ = try library.core.updateItem(id: id, patch: ItemPatch(notes: "lunch"))
        await library.undoLast()
        #expect(model.problem?.contains("changed in the meantime") == true)
        #expect(try library.word(id).notes == "lunch")
    }

    @Test func aWordBroughtBackFromTheTrashGoesBackOnUndo() async throws {
        let library = try TestLibrary()
        let model = library.model
        let id = try library.add("生词").id
        _ = try library.core.bulk(ids: [id], action: .trash)
        let trashed = try library.word(id)
        await model.reload()

        #expect(await model.save(WordDraft(simplified: "生词")))
        await library.settle()
        #expect(try library.word(id).lifecycle == .active)
        #expect(library.undo.undoActionName == "Add Word")

        await library.undoLast()
        #expect(try library.word(id).deletedAt == trashed.deletedAt, "back in the trash, as it was")
    }

    @Test func smartCollectionChangesAreUndoneAndRedone() async throws {
        let library = try TestLibrary()
        let model = library.model
        await model.reload()

        model.createSmartCollection("Drill", filter: LibraryFilter(view: .active, hskLevels: [1]))
        await library.settle()
        #expect(model.smartCollections.map(\.name) == ["Drill"])
        model.renameSmartCollection("Drill", to: "Practice")
        await library.settle()
        #expect(model.smartCollections.map(\.name) == ["Practice"])

        await library.undoLast()
        #expect(model.smartCollections.map(\.name) == ["Drill"])
        await library.undoLast()
        #expect(model.smartCollections.isEmpty)
        await library.redoLast()
        #expect(model.smartCollections.map(\.filter.hskLevels) == [[1]])
    }

    @Test func aSmartCollectionChangedSinceIsNotOverwritten() async throws {
        let library = try TestLibrary()
        let model = library.model
        await model.reload()
        model.createSmartCollection("Drill", filter: LibraryFilter(view: .active, hskLevels: [1]))
        await library.settle()

        // Changed by `shouci smart save` in the meantime
        _ = try library.core.updateSmartCollection(name: "Drill", filter: LibraryFilter(view: .active, hskLevels: [3]))
        await library.undoLast()
        #expect(model.problem?.contains("Drill changed in the meantime") == true)
        #expect(try library.core.smartCollections().map(\.filter.hskLevels) == [[3]])
    }

    @Test func undoingARenameReachesWordsTheWindowHadNotLoaded() async throws {
        let library = try TestLibrary()
        let model = library.model
        let shown = try library.add("学校", tags: ["drill"])
        await model.reload()
        // Tagged by `shouci` since the last reload
        let unseen = try library.add("旅行", tags: ["drill"])

        model.name(LibraryModel.Naming(purpose: .renameTag("drill"), text: "drilled"))
        await library.settle()
        #expect(try library.word(unseen.id).tags == ["drilled"])

        await library.undoLast()
        #expect(try library.word(shown.id).tags == ["drill"])
        #expect(try library.word(unseen.id).tags == ["drill"])
        #expect(model.tags.map(\.name) == ["drill"])
    }
}
