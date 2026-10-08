// The bridge, end to end: Swift calls into the Rust core and gets Swift
// values back. Behavior is tested in Rust; these check that it arrives.

import Foundation
import Testing

@testable import ShouciCore

/// A library in a temporary directory, with no dictionary and no downloads.
private func library() throws -> (Core, URL) {
    let dir = FileManager.default.temporaryDirectory
        .appendingPathComponent("shouci-swift-\(UUID().uuidString)")
    let core = try Core.open(
        config: CoreConfig(
            dataDir: dir.path,
            dictionariesDir: dir.appendingPathComponent("dictionaries").path,
            fetchDictionaries: false))
    return (core, dir)
}

@Test func errorsArriveWithTheirKindAndMessage() throws {
    let (core, dir) = try library()
    defer { try? FileManager.default.removeItem(at: dir) }

    #expect(core.dictionaryStatus() == .notLoaded)
    do {
        _ = try core.searchDictionary(query: "学校", kind: nil, limit: nil)
        Issue.record("search should fail before the dictionary loads")
    } catch let error as ShouciError {
        #expect(error.kind == .unavailable)
        #expect(error.message == "the dictionary is not loaded")
        #expect(describe(error) == error.message)
    }
}

@Test func wordsAreSavedEditedAndOrganized() throws {
    let (core, dir) = try library()
    defer { try? FileManager.default.removeItem(at: dir) }

    let saved = try core.addManual(
        word: ManualWord(
            simplified: "学校", pinyin: "xue2 xiao4", definition: "school", tags: ["class"]))
    #expect(saved.outcome == .inserted)
    #expect(saved.item.pinyinDisplay == "xué xiào")
    #expect(saved.item.verification == .confirmed)

    let edited = try core.updateItem(id: saved.item.id, patch: ItemPatch(notes: "came up in class"))
    #expect(edited.notes == "came up in class")

    let both = try core.editItem(
        id: saved.item.id, patch: ItemPatch(definition: "a school"),
        actions: [.addTags(["hsk"])])
    #expect(both.definition == "a school")
    #expect(both.tags == ["class", "hsk"])
    #expect(throws: ShouciError.self) {
        try core.editItem(id: saved.item.id, patch: ItemPatch(notes: "lost"), actions: [.addTags(["bad\tname"])])
    }
    #expect(try core.item(id: saved.item.id).notes == "came up in class", "all or nothing")

    _ = try core.bulk(ids: [saved.item.id], action: .addToCollection("Week 1"))
    #expect(try core.collections().map(\.name) == ["Week 1"])
    #expect(try core.tags().first?.count == 1)

    let found = try core.searchLibrary(query: "school", filter: LibraryFilter(), kind: nil, limit: nil)
    #expect(found.items.map(\.simplified) == ["学校"])

    _ = try core.bulk(ids: [saved.item.id], action: .trash)
    #expect(try core.listItems(filter: LibraryFilter()).isEmpty)
    #expect(try core.listItems(filter: LibraryFilter(view: .trash)).count == 1)
}

@Test func importAndExportGoThroughPreviews() throws {
    let (core, dir) = try library()
    defer { try? FileManager.default.removeItem(at: dir) }

    let source = dir.appendingPathComponent("from-pleco.txt")
    try "米饭\tmi3 fan4\tcooked rice\n".write(to: source, atomically: true, encoding: .utf8)
    let incoming = try core.previewImport(
        path: source.path, connector: "pleco", policy: .merge, force: false)
    #expect(incoming.view().counts.inserts == 1)
    #expect(try core.applyImport(preview: incoming).inserted == 1)

    let out = dir.appendingPathComponent("to-anki.txt")
    let outgoing = try core.previewExport(
        path: out.path, connector: "anki", request: ExportRequest(scope: .all))
    #expect(outgoing.view().words == ["米饭"])
    #expect(try core.applyExport(preview: outgoing).written == 1)
    #expect(try String(contentsOf: out, encoding: .utf8).contains("米饭"))

    let found = try core.detectImport(path: out.path, policy: .merge, force: false)
    #expect(found.preview.view().connectorId == "anki")
    #expect(found.unambiguous)
}

@Test func pinyinHelpers() {
    #expect(toneMarks(pinyin: "lv3 xing2") == "lǚ xíng")
    #expect(maxQueryChars() > 0)
}
