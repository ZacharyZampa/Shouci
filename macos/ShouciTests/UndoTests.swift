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

    @Test func aWordAddedByHandThatIsSavedAlreadyLosesWhatTheSaveGaveIt() async throws {
        let library = try TestLibrary()
        let model = library.model
        let other = try library.add("猫")
        let school = try library.core.addManual(
            word: ManualWord(simplified: "学校", pinyin: "xue2 xiao4", definition: "school")).item
        await model.reload()
        model.perform(.archive, on: [other.id])
        await library.settle()

        var draft = WordDraft(simplified: "学校")
        draft.pinyin = "xue2 xiao4"
        draft.tags = ["lunch"]
        draft.needsReview = true
        #expect(await model.save(draft))
        await library.settle()
        #expect(try library.word(school.id).tags == ["lunch"])
        #expect(library.undo.undoActionName == "Add Word", "not the archive before it")

        await library.undoLast()
        let undone = try library.word(school.id)
        #expect(undone.tags.isEmpty)
        #expect(undone.verification == .confirmed)
        #expect(try library.word(other.id).lifecycle == .archived, "the archive stays")

        await library.redoLast()
        #expect(try library.word(school.id).tags == ["lunch"])
    }

    @Test func savingAWordThatChangesNothingLeavesNothingToUndo() async throws {
        let library = try TestLibrary()
        let model = library.model
        let other = try library.add("猫")
        try library.add("学校")
        await model.reload()
        model.perform(.archive, on: [other.id])
        await library.settle()

        #expect(await model.save(WordDraft(simplified: "学校")))
        await library.settle()
        #expect(library.undo.undoActionName == "Archive")
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
